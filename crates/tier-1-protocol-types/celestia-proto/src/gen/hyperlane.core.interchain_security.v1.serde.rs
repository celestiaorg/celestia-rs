impl serde::Serialize for EventAnnounceStorageLocation {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.EventAnnounceStorageLocation", len)?;
        if true {
            struct_ser.serialize_field("mailbox_id", &self.mailbox_id)?;
        }
        if true {
            struct_ser.serialize_field("sender", &self.sender)?;
        }
        if true {
            struct_ser.serialize_field("validator", &self.validator)?;
        }
        if true {
            struct_ser.serialize_field("storage_location", &self.storage_location)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventAnnounceStorageLocation {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "mailbox_id",
            "mailboxId",
            "sender",
            "validator",
            "storage_location",
            "storageLocation",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MailboxId,
            Sender,
            Validator,
            StorageLocation,
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
                            "mailboxId" | "mailbox_id" => Ok(GeneratedField::MailboxId),
                            "sender" => Ok(GeneratedField::Sender),
                            "validator" => Ok(GeneratedField::Validator),
                            "storageLocation" | "storage_location" => Ok(GeneratedField::StorageLocation),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventAnnounceStorageLocation;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.EventAnnounceStorageLocation")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventAnnounceStorageLocation, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut mailbox_id__ = None;
                let mut sender__ = None;
                let mut validator__ = None;
                let mut storage_location__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::MailboxId => {
                            if mailbox_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailboxId"));
                            }
                            mailbox_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Sender => {
                            if sender__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sender"));
                            }
                            sender__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Validator => {
                            if validator__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validator"));
                            }
                            validator__ = Some(map_.next_value()?);
                        }
                        GeneratedField::StorageLocation => {
                            if storage_location__.is_some() {
                                return Err(serde::de::Error::duplicate_field("storageLocation"));
                            }
                            storage_location__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventAnnounceStorageLocation {
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                    sender: sender__.unwrap_or_default(),
                    validator: validator__.unwrap_or_default(),
                    storage_location: storage_location__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.EventAnnounceStorageLocation", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventCreateMerkleRootMultisigIsm {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.EventCreateMerkleRootMultisigIsm", len)?;
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("validators", &self.validators)?;
        }
        if true {
            struct_ser.serialize_field("threshold", &self.threshold)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventCreateMerkleRootMultisigIsm {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism_id",
            "ismId",
            "owner",
            "validators",
            "threshold",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IsmId,
            Owner,
            Validators,
            Threshold,
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
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
                            "owner" => Ok(GeneratedField::Owner),
                            "validators" => Ok(GeneratedField::Validators),
                            "threshold" => Ok(GeneratedField::Threshold),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventCreateMerkleRootMultisigIsm;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.EventCreateMerkleRootMultisigIsm")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventCreateMerkleRootMultisigIsm, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism_id__ = None;
                let mut owner__ = None;
                let mut validators__ = None;
                let mut threshold__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Validators => {
                            if validators__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validators"));
                            }
                            validators__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Threshold => {
                            if threshold__.is_some() {
                                return Err(serde::de::Error::duplicate_field("threshold"));
                            }
                            threshold__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(EventCreateMerkleRootMultisigIsm {
                    ism_id: ism_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    validators: validators__.unwrap_or_default(),
                    threshold: threshold__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.EventCreateMerkleRootMultisigIsm", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventCreateMessageIdMultisigIsm {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.EventCreateMessageIdMultisigIsm", len)?;
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("validators", &self.validators)?;
        }
        if true {
            struct_ser.serialize_field("threshold", &self.threshold)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventCreateMessageIdMultisigIsm {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism_id",
            "ismId",
            "owner",
            "validators",
            "threshold",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IsmId,
            Owner,
            Validators,
            Threshold,
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
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
                            "owner" => Ok(GeneratedField::Owner),
                            "validators" => Ok(GeneratedField::Validators),
                            "threshold" => Ok(GeneratedField::Threshold),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventCreateMessageIdMultisigIsm;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.EventCreateMessageIdMultisigIsm")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventCreateMessageIdMultisigIsm, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism_id__ = None;
                let mut owner__ = None;
                let mut validators__ = None;
                let mut threshold__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Validators => {
                            if validators__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validators"));
                            }
                            validators__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Threshold => {
                            if threshold__.is_some() {
                                return Err(serde::de::Error::duplicate_field("threshold"));
                            }
                            threshold__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(EventCreateMessageIdMultisigIsm {
                    ism_id: ism_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    validators: validators__.unwrap_or_default(),
                    threshold: threshold__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.EventCreateMessageIdMultisigIsm", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventCreateNoopIsm {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.EventCreateNoopIsm", len)?;
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventCreateNoopIsm {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism_id",
            "ismId",
            "owner",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IsmId,
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
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
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
            type Value = EventCreateNoopIsm;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.EventCreateNoopIsm")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventCreateNoopIsm, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism_id__ = None;
                let mut owner__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventCreateNoopIsm {
                    ism_id: ism_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.EventCreateNoopIsm", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventCreateRoutingIsm {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.EventCreateRoutingIsm", len)?;
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventCreateRoutingIsm {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism_id",
            "ismId",
            "owner",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IsmId,
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
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
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
            type Value = EventCreateRoutingIsm;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.EventCreateRoutingIsm")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventCreateRoutingIsm, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism_id__ = None;
                let mut owner__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventCreateRoutingIsm {
                    ism_id: ism_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.EventCreateRoutingIsm", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventRemoveRoutingIsmDomain {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.EventRemoveRoutingIsmDomain", len)?;
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("route_domain", &self.route_domain)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventRemoveRoutingIsmDomain {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism_id",
            "ismId",
            "owner",
            "route_domain",
            "routeDomain",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IsmId,
            Owner,
            RouteDomain,
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
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
                            "owner" => Ok(GeneratedField::Owner),
                            "routeDomain" | "route_domain" => Ok(GeneratedField::RouteDomain),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventRemoveRoutingIsmDomain;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.EventRemoveRoutingIsmDomain")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventRemoveRoutingIsmDomain, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism_id__ = None;
                let mut owner__ = None;
                let mut route_domain__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RouteDomain => {
                            if route_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("routeDomain"));
                            }
                            route_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(EventRemoveRoutingIsmDomain {
                    ism_id: ism_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    route_domain: route_domain__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.EventRemoveRoutingIsmDomain", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventSetRoutingIsm {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.EventSetRoutingIsm", len)?;
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
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
impl<'de> serde::Deserialize<'de> for EventSetRoutingIsm {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism_id",
            "ismId",
            "owner",
            "new_owner",
            "newOwner",
            "renounce_ownership",
            "renounceOwnership",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IsmId,
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
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
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
            type Value = EventSetRoutingIsm;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.EventSetRoutingIsm")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventSetRoutingIsm, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism_id__ = None;
                let mut owner__ = None;
                let mut new_owner__ = None;
                let mut renounce_ownership__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
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
                Ok(EventSetRoutingIsm {
                    ism_id: ism_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    new_owner: new_owner__.unwrap_or_default(),
                    renounce_ownership: renounce_ownership__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.EventSetRoutingIsm", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventSetRoutingIsmDomain {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.EventSetRoutingIsmDomain", len)?;
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("route_ism_id", &self.route_ism_id)?;
        }
        if true {
            struct_ser.serialize_field("route_domain", &self.route_domain)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventSetRoutingIsmDomain {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism_id",
            "ismId",
            "owner",
            "route_ism_id",
            "routeIsmId",
            "route_domain",
            "routeDomain",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IsmId,
            Owner,
            RouteIsmId,
            RouteDomain,
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
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
                            "owner" => Ok(GeneratedField::Owner),
                            "routeIsmId" | "route_ism_id" => Ok(GeneratedField::RouteIsmId),
                            "routeDomain" | "route_domain" => Ok(GeneratedField::RouteDomain),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventSetRoutingIsmDomain;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.EventSetRoutingIsmDomain")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventSetRoutingIsmDomain, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism_id__ = None;
                let mut owner__ = None;
                let mut route_ism_id__ = None;
                let mut route_domain__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RouteIsmId => {
                            if route_ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("routeIsmId"));
                            }
                            route_ism_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RouteDomain => {
                            if route_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("routeDomain"));
                            }
                            route_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(EventSetRoutingIsmDomain {
                    ism_id: ism_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    route_ism_id: route_ism_id__.unwrap_or_default(),
                    route_domain: route_domain__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.EventSetRoutingIsmDomain", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.GenesisState", len)?;
        if true {
            struct_ser.serialize_field("isms", &self.isms)?;
        }
        if true {
            struct_ser.serialize_field("validator_storage_locations", &self.validator_storage_locations)?;
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
            "isms",
            "validator_storage_locations",
            "validatorStorageLocations",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Isms,
            ValidatorStorageLocations,
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
                            "isms" => Ok(GeneratedField::Isms),
                            "validatorStorageLocations" | "validator_storage_locations" => Ok(GeneratedField::ValidatorStorageLocations),
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
                formatter.write_str("struct hyperlane.core.interchain_security.v1.GenesisState")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<GenesisState, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut isms__ = None;
                let mut validator_storage_locations__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Isms => {
                            if isms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("isms"));
                            }
                            isms__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ValidatorStorageLocations => {
                            if validator_storage_locations__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validatorStorageLocations"));
                            }
                            validator_storage_locations__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(GenesisState {
                    isms: isms__.unwrap_or_default(),
                    validator_storage_locations: validator_storage_locations__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.GenesisState", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for GenesisValidatorStorageLocationWrapper {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.GenesisValidatorStorageLocationWrapper", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("mailbox_id", ToString::to_string(&self.mailbox_id).as_str())?;
        }
        if true {
            struct_ser.serialize_field("validator_address", &self.validator_address)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("index", ToString::to_string(&self.index).as_str())?;
        }
        if true {
            struct_ser.serialize_field("storage_location", &self.storage_location)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for GenesisValidatorStorageLocationWrapper {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "mailbox_id",
            "mailboxId",
            "validator_address",
            "validatorAddress",
            "index",
            "storage_location",
            "storageLocation",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MailboxId,
            ValidatorAddress,
            Index,
            StorageLocation,
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
                            "mailboxId" | "mailbox_id" => Ok(GeneratedField::MailboxId),
                            "validatorAddress" | "validator_address" => Ok(GeneratedField::ValidatorAddress),
                            "index" => Ok(GeneratedField::Index),
                            "storageLocation" | "storage_location" => Ok(GeneratedField::StorageLocation),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = GenesisValidatorStorageLocationWrapper;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.GenesisValidatorStorageLocationWrapper")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<GenesisValidatorStorageLocationWrapper, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut mailbox_id__ = None;
                let mut validator_address__ = None;
                let mut index__ = None;
                let mut storage_location__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::MailboxId => {
                            if mailbox_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailboxId"));
                            }
                            mailbox_id__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ValidatorAddress => {
                            if validator_address__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validatorAddress"));
                            }
                            validator_address__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Index => {
                            if index__.is_some() {
                                return Err(serde::de::Error::duplicate_field("index"));
                            }
                            index__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::StorageLocation => {
                            if storage_location__.is_some() {
                                return Err(serde::de::Error::duplicate_field("storageLocation"));
                            }
                            storage_location__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(GenesisValidatorStorageLocationWrapper {
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                    validator_address: validator_address__.unwrap_or_default(),
                    index: index__.unwrap_or_default(),
                    storage_location: storage_location__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.GenesisValidatorStorageLocationWrapper", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MerkleRootMultisigIsm {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MerkleRootMultisigISM", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("validators", &self.validators)?;
        }
        if true {
            struct_ser.serialize_field("threshold", &self.threshold)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MerkleRootMultisigIsm {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
            "owner",
            "validators",
            "threshold",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
            Owner,
            Validators,
            Threshold,
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
                            "validators" => Ok(GeneratedField::Validators),
                            "threshold" => Ok(GeneratedField::Threshold),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MerkleRootMultisigIsm;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MerkleRootMultisigISM")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MerkleRootMultisigIsm, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                let mut owner__ = None;
                let mut validators__ = None;
                let mut threshold__ = None;
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
                        GeneratedField::Validators => {
                            if validators__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validators"));
                            }
                            validators__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Threshold => {
                            if threshold__.is_some() {
                                return Err(serde::de::Error::duplicate_field("threshold"));
                            }
                            threshold__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(MerkleRootMultisigIsm {
                    id: id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    validators: validators__.unwrap_or_default(),
                    threshold: threshold__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MerkleRootMultisigISM", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MessageIdMultisigIsm {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MessageIdMultisigISM", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("validators", &self.validators)?;
        }
        if true {
            struct_ser.serialize_field("threshold", &self.threshold)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MessageIdMultisigIsm {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
            "owner",
            "validators",
            "threshold",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
            Owner,
            Validators,
            Threshold,
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
                            "validators" => Ok(GeneratedField::Validators),
                            "threshold" => Ok(GeneratedField::Threshold),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MessageIdMultisigIsm;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MessageIdMultisigISM")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MessageIdMultisigIsm, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                let mut owner__ = None;
                let mut validators__ = None;
                let mut threshold__ = None;
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
                        GeneratedField::Validators => {
                            if validators__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validators"));
                            }
                            validators__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Threshold => {
                            if threshold__.is_some() {
                                return Err(serde::de::Error::duplicate_field("threshold"));
                            }
                            threshold__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(MessageIdMultisigIsm {
                    id: id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    validators: validators__.unwrap_or_default(),
                    threshold: threshold__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MessageIdMultisigISM", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgAnnounceValidator {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgAnnounceValidator", len)?;
        if true {
            struct_ser.serialize_field("validator", &self.validator)?;
        }
        if true {
            struct_ser.serialize_field("storage_location", &self.storage_location)?;
        }
        if true {
            struct_ser.serialize_field("signature", &self.signature)?;
        }
        if true {
            struct_ser.serialize_field("mailbox_id", &self.mailbox_id)?;
        }
        if true {
            struct_ser.serialize_field("creator", &self.creator)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgAnnounceValidator {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "validator",
            "storage_location",
            "storageLocation",
            "signature",
            "mailbox_id",
            "mailboxId",
            "creator",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Validator,
            StorageLocation,
            Signature,
            MailboxId,
            Creator,
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
                            "validator" => Ok(GeneratedField::Validator),
                            "storageLocation" | "storage_location" => Ok(GeneratedField::StorageLocation),
                            "signature" => Ok(GeneratedField::Signature),
                            "mailboxId" | "mailbox_id" => Ok(GeneratedField::MailboxId),
                            "creator" => Ok(GeneratedField::Creator),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgAnnounceValidator;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgAnnounceValidator")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgAnnounceValidator, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut validator__ = None;
                let mut storage_location__ = None;
                let mut signature__ = None;
                let mut mailbox_id__ = None;
                let mut creator__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Validator => {
                            if validator__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validator"));
                            }
                            validator__ = Some(map_.next_value()?);
                        }
                        GeneratedField::StorageLocation => {
                            if storage_location__.is_some() {
                                return Err(serde::de::Error::duplicate_field("storageLocation"));
                            }
                            storage_location__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Signature => {
                            if signature__.is_some() {
                                return Err(serde::de::Error::duplicate_field("signature"));
                            }
                            signature__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MailboxId => {
                            if mailbox_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailboxId"));
                            }
                            mailbox_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Creator => {
                            if creator__.is_some() {
                                return Err(serde::de::Error::duplicate_field("creator"));
                            }
                            creator__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgAnnounceValidator {
                    validator: validator__.unwrap_or_default(),
                    storage_location: storage_location__.unwrap_or_default(),
                    signature: signature__.unwrap_or_default(),
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                    creator: creator__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgAnnounceValidator", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgAnnounceValidatorResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgAnnounceValidatorResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgAnnounceValidatorResponse {
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
            type Value = MsgAnnounceValidatorResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgAnnounceValidatorResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgAnnounceValidatorResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(MsgAnnounceValidatorResponse {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgAnnounceValidatorResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateMerkleRootMultisigIsm {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsm", len)?;
        if true {
            struct_ser.serialize_field("creator", &self.creator)?;
        }
        if true {
            struct_ser.serialize_field("validators", &self.validators)?;
        }
        if true {
            struct_ser.serialize_field("threshold", &self.threshold)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateMerkleRootMultisigIsm {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "creator",
            "validators",
            "threshold",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Creator,
            Validators,
            Threshold,
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
                            "creator" => Ok(GeneratedField::Creator),
                            "validators" => Ok(GeneratedField::Validators),
                            "threshold" => Ok(GeneratedField::Threshold),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateMerkleRootMultisigIsm;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsm")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateMerkleRootMultisigIsm, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut creator__ = None;
                let mut validators__ = None;
                let mut threshold__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Creator => {
                            if creator__.is_some() {
                                return Err(serde::de::Error::duplicate_field("creator"));
                            }
                            creator__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Validators => {
                            if validators__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validators"));
                            }
                            validators__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Threshold => {
                            if threshold__.is_some() {
                                return Err(serde::de::Error::duplicate_field("threshold"));
                            }
                            threshold__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(MsgCreateMerkleRootMultisigIsm {
                    creator: creator__.unwrap_or_default(),
                    validators: validators__.unwrap_or_default(),
                    threshold: threshold__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsm", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateMerkleRootMultisigIsmResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsmResponse", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateMerkleRootMultisigIsmResponse {
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
            type Value = MsgCreateMerkleRootMultisigIsmResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsmResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateMerkleRootMultisigIsmResponse, V::Error>
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
                Ok(MsgCreateMerkleRootMultisigIsmResponse {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsmResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateMessageIdMultisigIsm {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgCreateMessageIdMultisigIsm", len)?;
        if true {
            struct_ser.serialize_field("creator", &self.creator)?;
        }
        if true {
            struct_ser.serialize_field("validators", &self.validators)?;
        }
        if true {
            struct_ser.serialize_field("threshold", &self.threshold)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateMessageIdMultisigIsm {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "creator",
            "validators",
            "threshold",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Creator,
            Validators,
            Threshold,
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
                            "creator" => Ok(GeneratedField::Creator),
                            "validators" => Ok(GeneratedField::Validators),
                            "threshold" => Ok(GeneratedField::Threshold),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateMessageIdMultisigIsm;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgCreateMessageIdMultisigIsm")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateMessageIdMultisigIsm, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut creator__ = None;
                let mut validators__ = None;
                let mut threshold__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Creator => {
                            if creator__.is_some() {
                                return Err(serde::de::Error::duplicate_field("creator"));
                            }
                            creator__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Validators => {
                            if validators__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validators"));
                            }
                            validators__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Threshold => {
                            if threshold__.is_some() {
                                return Err(serde::de::Error::duplicate_field("threshold"));
                            }
                            threshold__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(MsgCreateMessageIdMultisigIsm {
                    creator: creator__.unwrap_or_default(),
                    validators: validators__.unwrap_or_default(),
                    threshold: threshold__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgCreateMessageIdMultisigIsm", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateMessageIdMultisigIsmResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgCreateMessageIdMultisigIsmResponse", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateMessageIdMultisigIsmResponse {
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
            type Value = MsgCreateMessageIdMultisigIsmResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgCreateMessageIdMultisigIsmResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateMessageIdMultisigIsmResponse, V::Error>
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
                Ok(MsgCreateMessageIdMultisigIsmResponse {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgCreateMessageIdMultisigIsmResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateNoopIsm {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgCreateNoopIsm", len)?;
        if true {
            struct_ser.serialize_field("creator", &self.creator)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateNoopIsm {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "creator",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Creator,
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
                            "creator" => Ok(GeneratedField::Creator),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateNoopIsm;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgCreateNoopIsm")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateNoopIsm, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut creator__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Creator => {
                            if creator__.is_some() {
                                return Err(serde::de::Error::duplicate_field("creator"));
                            }
                            creator__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgCreateNoopIsm {
                    creator: creator__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgCreateNoopIsm", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateNoopIsmResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgCreateNoopIsmResponse", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateNoopIsmResponse {
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
            type Value = MsgCreateNoopIsmResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgCreateNoopIsmResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateNoopIsmResponse, V::Error>
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
                Ok(MsgCreateNoopIsmResponse {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgCreateNoopIsmResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateRoutingIsm {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgCreateRoutingIsm", len)?;
        if true {
            struct_ser.serialize_field("creator", &self.creator)?;
        }
        if true {
            struct_ser.serialize_field("routes", &self.routes)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateRoutingIsm {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "creator",
            "routes",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Creator,
            Routes,
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
                            "creator" => Ok(GeneratedField::Creator),
                            "routes" => Ok(GeneratedField::Routes),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateRoutingIsm;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgCreateRoutingIsm")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateRoutingIsm, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut creator__ = None;
                let mut routes__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Creator => {
                            if creator__.is_some() {
                                return Err(serde::de::Error::duplicate_field("creator"));
                            }
                            creator__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Routes => {
                            if routes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("routes"));
                            }
                            routes__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgCreateRoutingIsm {
                    creator: creator__.unwrap_or_default(),
                    routes: routes__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgCreateRoutingIsm", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateRoutingIsmResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgCreateRoutingIsmResponse", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateRoutingIsmResponse {
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
            type Value = MsgCreateRoutingIsmResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgCreateRoutingIsmResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateRoutingIsmResponse, V::Error>
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
                Ok(MsgCreateRoutingIsmResponse {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgCreateRoutingIsmResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgRemoveRoutingIsmDomain {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgRemoveRoutingIsmDomain", len)?;
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
        }
        if true {
            struct_ser.serialize_field("domain", &self.domain)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgRemoveRoutingIsmDomain {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism_id",
            "ismId",
            "domain",
            "owner",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IsmId,
            Domain,
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
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
                            "domain" => Ok(GeneratedField::Domain),
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
            type Value = MsgRemoveRoutingIsmDomain;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgRemoveRoutingIsmDomain")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgRemoveRoutingIsmDomain, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism_id__ = None;
                let mut domain__ = None;
                let mut owner__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Domain => {
                            if domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("domain"));
                            }
                            domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgRemoveRoutingIsmDomain {
                    ism_id: ism_id__.unwrap_or_default(),
                    domain: domain__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgRemoveRoutingIsmDomain", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgRemoveRoutingIsmDomainResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgRemoveRoutingIsmDomainResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgRemoveRoutingIsmDomainResponse {
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
            type Value = MsgRemoveRoutingIsmDomainResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgRemoveRoutingIsmDomainResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgRemoveRoutingIsmDomainResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(MsgRemoveRoutingIsmDomainResponse {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgRemoveRoutingIsmDomainResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgSetRoutingIsmDomain {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomain", len)?;
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
        }
        if let Some(v) = self.route.as_ref() {
            struct_ser.serialize_field("route", v)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgSetRoutingIsmDomain {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism_id",
            "ismId",
            "route",
            "owner",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IsmId,
            Route,
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
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
                            "route" => Ok(GeneratedField::Route),
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
            type Value = MsgSetRoutingIsmDomain;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomain")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgSetRoutingIsmDomain, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism_id__ = None;
                let mut route__ = None;
                let mut owner__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Route => {
                            if route__.is_some() {
                                return Err(serde::de::Error::duplicate_field("route"));
                            }
                            route__ = map_.next_value()?;
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgSetRoutingIsmDomain {
                    ism_id: ism_id__.unwrap_or_default(),
                    route: route__,
                    owner: owner__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomain", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgSetRoutingIsmDomainResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomainResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgSetRoutingIsmDomainResponse {
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
            type Value = MsgSetRoutingIsmDomainResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomainResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgSetRoutingIsmDomainResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(MsgSetRoutingIsmDomainResponse {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomainResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgUpdateRoutingIsmOwner {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwner", len)?;
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
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
impl<'de> serde::Deserialize<'de> for MsgUpdateRoutingIsmOwner {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism_id",
            "ismId",
            "owner",
            "new_owner",
            "newOwner",
            "renounce_ownership",
            "renounceOwnership",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IsmId,
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
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
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
            type Value = MsgUpdateRoutingIsmOwner;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwner")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgUpdateRoutingIsmOwner, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism_id__ = None;
                let mut owner__ = None;
                let mut new_owner__ = None;
                let mut renounce_ownership__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
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
                Ok(MsgUpdateRoutingIsmOwner {
                    ism_id: ism_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    new_owner: new_owner__.unwrap_or_default(),
                    renounce_ownership: renounce_ownership__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwner", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgUpdateRoutingIsmOwnerResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwnerResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgUpdateRoutingIsmOwnerResponse {
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
            type Value = MsgUpdateRoutingIsmOwnerResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwnerResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgUpdateRoutingIsmOwnerResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(MsgUpdateRoutingIsmOwnerResponse {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwnerResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for NoopIsm {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.NoopISM", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for NoopIsm {
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
            type Value = NoopIsm;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.NoopISM")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<NoopIsm, V::Error>
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
                Ok(NoopIsm {
                    id: id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.NoopISM", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryAnnouncedStorageLocationsRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.QueryAnnouncedStorageLocationsRequest", len)?;
        if true {
            struct_ser.serialize_field("mailbox_id", &self.mailbox_id)?;
        }
        if true {
            struct_ser.serialize_field("validator_address", &self.validator_address)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryAnnouncedStorageLocationsRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "mailbox_id",
            "mailboxId",
            "validator_address",
            "validatorAddress",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MailboxId,
            ValidatorAddress,
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
                            "mailboxId" | "mailbox_id" => Ok(GeneratedField::MailboxId),
                            "validatorAddress" | "validator_address" => Ok(GeneratedField::ValidatorAddress),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryAnnouncedStorageLocationsRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.QueryAnnouncedStorageLocationsRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryAnnouncedStorageLocationsRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut mailbox_id__ = None;
                let mut validator_address__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::MailboxId => {
                            if mailbox_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailboxId"));
                            }
                            mailbox_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ValidatorAddress => {
                            if validator_address__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validatorAddress"));
                            }
                            validator_address__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryAnnouncedStorageLocationsRequest {
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                    validator_address: validator_address__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.QueryAnnouncedStorageLocationsRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryAnnouncedStorageLocationsResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.QueryAnnouncedStorageLocationsResponse", len)?;
        if true {
            struct_ser.serialize_field("storage_locations", &self.storage_locations)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryAnnouncedStorageLocationsResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "storage_locations",
            "storageLocations",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            StorageLocations,
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
                            "storageLocations" | "storage_locations" => Ok(GeneratedField::StorageLocations),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryAnnouncedStorageLocationsResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.QueryAnnouncedStorageLocationsResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryAnnouncedStorageLocationsResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut storage_locations__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::StorageLocations => {
                            if storage_locations__.is_some() {
                                return Err(serde::de::Error::duplicate_field("storageLocations"));
                            }
                            storage_locations__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryAnnouncedStorageLocationsResponse {
                    storage_locations: storage_locations__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.QueryAnnouncedStorageLocationsResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryIsmRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.QueryIsmRequest", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryIsmRequest {
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
            type Value = QueryIsmRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.QueryIsmRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryIsmRequest, V::Error>
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
                Ok(QueryIsmRequest {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.QueryIsmRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryIsmResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.QueryIsmResponse", len)?;
        if let Some(v) = self.ism.as_ref() {
            struct_ser.serialize_field("ism", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryIsmResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Ism,
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
                            "ism" => Ok(GeneratedField::Ism),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryIsmResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.QueryIsmResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryIsmResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Ism => {
                            if ism__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ism"));
                            }
                            ism__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryIsmResponse {
                    ism: ism__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.QueryIsmResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryIsmsRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.QueryIsmsRequest", len)?;
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryIsmsRequest {
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
            type Value = QueryIsmsRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.QueryIsmsRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryIsmsRequest, V::Error>
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
                Ok(QueryIsmsRequest {
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.QueryIsmsRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryIsmsResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.QueryIsmsResponse", len)?;
        if true {
            struct_ser.serialize_field("isms", &self.isms)?;
        }
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryIsmsResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "isms",
            "pagination",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Isms,
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
                            "isms" => Ok(GeneratedField::Isms),
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
            type Value = QueryIsmsResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.QueryIsmsResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryIsmsResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut isms__ = None;
                let mut pagination__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Isms => {
                            if isms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("isms"));
                            }
                            isms__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Pagination => {
                            if pagination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pagination"));
                            }
                            pagination__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryIsmsResponse {
                    isms: isms__.unwrap_or_default(),
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.QueryIsmsResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryLatestAnnouncedStorageLocationRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.QueryLatestAnnouncedStorageLocationRequest", len)?;
        if true {
            struct_ser.serialize_field("mailbox_id", &self.mailbox_id)?;
        }
        if true {
            struct_ser.serialize_field("validator_address", &self.validator_address)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryLatestAnnouncedStorageLocationRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "mailbox_id",
            "mailboxId",
            "validator_address",
            "validatorAddress",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MailboxId,
            ValidatorAddress,
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
                            "mailboxId" | "mailbox_id" => Ok(GeneratedField::MailboxId),
                            "validatorAddress" | "validator_address" => Ok(GeneratedField::ValidatorAddress),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryLatestAnnouncedStorageLocationRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.QueryLatestAnnouncedStorageLocationRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryLatestAnnouncedStorageLocationRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut mailbox_id__ = None;
                let mut validator_address__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::MailboxId => {
                            if mailbox_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailboxId"));
                            }
                            mailbox_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ValidatorAddress => {
                            if validator_address__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validatorAddress"));
                            }
                            validator_address__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryLatestAnnouncedStorageLocationRequest {
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                    validator_address: validator_address__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.QueryLatestAnnouncedStorageLocationRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryLatestAnnouncedStorageLocationResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.QueryLatestAnnouncedStorageLocationResponse", len)?;
        if true {
            struct_ser.serialize_field("storage_location", &self.storage_location)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryLatestAnnouncedStorageLocationResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "storage_location",
            "storageLocation",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            StorageLocation,
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
                            "storageLocation" | "storage_location" => Ok(GeneratedField::StorageLocation),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryLatestAnnouncedStorageLocationResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.QueryLatestAnnouncedStorageLocationResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryLatestAnnouncedStorageLocationResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut storage_location__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::StorageLocation => {
                            if storage_location__.is_some() {
                                return Err(serde::de::Error::duplicate_field("storageLocation"));
                            }
                            storage_location__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryLatestAnnouncedStorageLocationResponse {
                    storage_location: storage_location__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.QueryLatestAnnouncedStorageLocationResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Route {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.Route", len)?;
        if true {
            struct_ser.serialize_field("ism", &self.ism)?;
        }
        if true {
            struct_ser.serialize_field("domain", &self.domain)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Route {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism",
            "domain",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Ism,
            Domain,
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
                            "ism" => Ok(GeneratedField::Ism),
                            "domain" => Ok(GeneratedField::Domain),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Route;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.Route")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Route, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism__ = None;
                let mut domain__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Ism => {
                            if ism__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ism"));
                            }
                            ism__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Domain => {
                            if domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("domain"));
                            }
                            domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(Route {
                    ism: ism__.unwrap_or_default(),
                    domain: domain__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.Route", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for RoutingIsm {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.interchain_security.v1.RoutingISM", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("routes", &self.routes)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for RoutingIsm {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
            "owner",
            "routes",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
            Owner,
            Routes,
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
                            "routes" => Ok(GeneratedField::Routes),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = RoutingIsm;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.interchain_security.v1.RoutingISM")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<RoutingIsm, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                let mut owner__ = None;
                let mut routes__ = None;
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
                        GeneratedField::Routes => {
                            if routes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("routes"));
                            }
                            routes__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(RoutingIsm {
                    id: id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    routes: routes__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.interchain_security.v1.RoutingISM", FIELDS, GeneratedVisitor)
    }
}
