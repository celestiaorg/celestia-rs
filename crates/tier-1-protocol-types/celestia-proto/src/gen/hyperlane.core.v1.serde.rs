impl serde::Serialize for EventCreateMailbox {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.EventCreateMailbox", len)?;
        if true {
            struct_ser.serialize_field("mailbox_id", &self.mailbox_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("default_ism", &self.default_ism)?;
        }
        if true {
            struct_ser.serialize_field("default_hook", &self.default_hook)?;
        }
        if true {
            struct_ser.serialize_field("required_hook", &self.required_hook)?;
        }
        if true {
            struct_ser.serialize_field("local_domain", &self.local_domain)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventCreateMailbox {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "mailbox_id",
            "mailboxId",
            "owner",
            "default_ism",
            "defaultIsm",
            "default_hook",
            "defaultHook",
            "required_hook",
            "requiredHook",
            "local_domain",
            "localDomain",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MailboxId,
            Owner,
            DefaultIsm,
            DefaultHook,
            RequiredHook,
            LocalDomain,
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
                            "owner" => Ok(GeneratedField::Owner),
                            "defaultIsm" | "default_ism" => Ok(GeneratedField::DefaultIsm),
                            "defaultHook" | "default_hook" => Ok(GeneratedField::DefaultHook),
                            "requiredHook" | "required_hook" => Ok(GeneratedField::RequiredHook),
                            "localDomain" | "local_domain" => Ok(GeneratedField::LocalDomain),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventCreateMailbox;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.EventCreateMailbox")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventCreateMailbox, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut mailbox_id__ = None;
                let mut owner__ = None;
                let mut default_ism__ = None;
                let mut default_hook__ = None;
                let mut required_hook__ = None;
                let mut local_domain__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
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
                        GeneratedField::DefaultIsm => {
                            if default_ism__.is_some() {
                                return Err(serde::de::Error::duplicate_field("defaultIsm"));
                            }
                            default_ism__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DefaultHook => {
                            if default_hook__.is_some() {
                                return Err(serde::de::Error::duplicate_field("defaultHook"));
                            }
                            default_hook__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RequiredHook => {
                            if required_hook__.is_some() {
                                return Err(serde::de::Error::duplicate_field("requiredHook"));
                            }
                            required_hook__ = Some(map_.next_value()?);
                        }
                        GeneratedField::LocalDomain => {
                            if local_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("localDomain"));
                            }
                            local_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(EventCreateMailbox {
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    default_ism: default_ism__.unwrap_or_default(),
                    default_hook: default_hook__.unwrap_or_default(),
                    required_hook: required_hook__.unwrap_or_default(),
                    local_domain: local_domain__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.EventCreateMailbox", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventDispatch {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.EventDispatch", len)?;
        if true {
            struct_ser.serialize_field("origin_mailbox_id", &self.origin_mailbox_id)?;
        }
        if true {
            struct_ser.serialize_field("sender", &self.sender)?;
        }
        if true {
            struct_ser.serialize_field("destination", &self.destination)?;
        }
        if true {
            struct_ser.serialize_field("recipient", &self.recipient)?;
        }
        if true {
            struct_ser.serialize_field("message", &self.message)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventDispatch {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "origin_mailbox_id",
            "originMailboxId",
            "sender",
            "destination",
            "recipient",
            "message",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            OriginMailboxId,
            Sender,
            Destination,
            Recipient,
            Message,
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
                            "originMailboxId" | "origin_mailbox_id" => Ok(GeneratedField::OriginMailboxId),
                            "sender" => Ok(GeneratedField::Sender),
                            "destination" => Ok(GeneratedField::Destination),
                            "recipient" => Ok(GeneratedField::Recipient),
                            "message" => Ok(GeneratedField::Message),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventDispatch;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.EventDispatch")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventDispatch, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut origin_mailbox_id__ = None;
                let mut sender__ = None;
                let mut destination__ = None;
                let mut recipient__ = None;
                let mut message__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::OriginMailboxId => {
                            if origin_mailbox_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originMailboxId"));
                            }
                            origin_mailbox_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Sender => {
                            if sender__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sender"));
                            }
                            sender__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Destination => {
                            if destination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destination"));
                            }
                            destination__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Recipient => {
                            if recipient__.is_some() {
                                return Err(serde::de::Error::duplicate_field("recipient"));
                            }
                            recipient__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Message => {
                            if message__.is_some() {
                                return Err(serde::de::Error::duplicate_field("message"));
                            }
                            message__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventDispatch {
                    origin_mailbox_id: origin_mailbox_id__.unwrap_or_default(),
                    sender: sender__.unwrap_or_default(),
                    destination: destination__.unwrap_or_default(),
                    recipient: recipient__.unwrap_or_default(),
                    message: message__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.EventDispatch", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventProcess {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.EventProcess", len)?;
        if true {
            struct_ser.serialize_field("origin_mailbox_id", &self.origin_mailbox_id)?;
        }
        if true {
            struct_ser.serialize_field("origin", &self.origin)?;
        }
        if true {
            struct_ser.serialize_field("sender", &self.sender)?;
        }
        if true {
            struct_ser.serialize_field("recipient", &self.recipient)?;
        }
        if true {
            struct_ser.serialize_field("message_id", &self.message_id)?;
        }
        if true {
            struct_ser.serialize_field("message", &self.message)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventProcess {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "origin_mailbox_id",
            "originMailboxId",
            "origin",
            "sender",
            "recipient",
            "message_id",
            "messageId",
            "message",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            OriginMailboxId,
            Origin,
            Sender,
            Recipient,
            MessageId,
            Message,
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
                            "originMailboxId" | "origin_mailbox_id" => Ok(GeneratedField::OriginMailboxId),
                            "origin" => Ok(GeneratedField::Origin),
                            "sender" => Ok(GeneratedField::Sender),
                            "recipient" => Ok(GeneratedField::Recipient),
                            "messageId" | "message_id" => Ok(GeneratedField::MessageId),
                            "message" => Ok(GeneratedField::Message),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventProcess;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.EventProcess")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventProcess, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut origin_mailbox_id__ = None;
                let mut origin__ = None;
                let mut sender__ = None;
                let mut recipient__ = None;
                let mut message_id__ = None;
                let mut message__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::OriginMailboxId => {
                            if origin_mailbox_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originMailboxId"));
                            }
                            origin_mailbox_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Origin => {
                            if origin__.is_some() {
                                return Err(serde::de::Error::duplicate_field("origin"));
                            }
                            origin__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Sender => {
                            if sender__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sender"));
                            }
                            sender__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Recipient => {
                            if recipient__.is_some() {
                                return Err(serde::de::Error::duplicate_field("recipient"));
                            }
                            recipient__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MessageId => {
                            if message_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("messageId"));
                            }
                            message_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Message => {
                            if message__.is_some() {
                                return Err(serde::de::Error::duplicate_field("message"));
                            }
                            message__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventProcess {
                    origin_mailbox_id: origin_mailbox_id__.unwrap_or_default(),
                    origin: origin__.unwrap_or_default(),
                    sender: sender__.unwrap_or_default(),
                    recipient: recipient__.unwrap_or_default(),
                    message_id: message_id__.unwrap_or_default(),
                    message: message__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.EventProcess", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventSetMailbox {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.EventSetMailbox", len)?;
        if true {
            struct_ser.serialize_field("mailbox_id", &self.mailbox_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("default_ism", &self.default_ism)?;
        }
        if true {
            struct_ser.serialize_field("default_hook", &self.default_hook)?;
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
impl<'de> serde::Deserialize<'de> for EventSetMailbox {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "mailbox_id",
            "mailboxId",
            "owner",
            "default_ism",
            "defaultIsm",
            "default_hook",
            "defaultHook",
            "new_owner",
            "newOwner",
            "renounce_ownership",
            "renounceOwnership",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MailboxId,
            Owner,
            DefaultIsm,
            DefaultHook,
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
                            "mailboxId" | "mailbox_id" => Ok(GeneratedField::MailboxId),
                            "owner" => Ok(GeneratedField::Owner),
                            "defaultIsm" | "default_ism" => Ok(GeneratedField::DefaultIsm),
                            "defaultHook" | "default_hook" => Ok(GeneratedField::DefaultHook),
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
            type Value = EventSetMailbox;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.EventSetMailbox")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventSetMailbox, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut mailbox_id__ = None;
                let mut owner__ = None;
                let mut default_ism__ = None;
                let mut default_hook__ = None;
                let mut new_owner__ = None;
                let mut renounce_ownership__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
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
                        GeneratedField::DefaultIsm => {
                            if default_ism__.is_some() {
                                return Err(serde::de::Error::duplicate_field("defaultIsm"));
                            }
                            default_ism__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DefaultHook => {
                            if default_hook__.is_some() {
                                return Err(serde::de::Error::duplicate_field("defaultHook"));
                            }
                            default_hook__ = Some(map_.next_value()?);
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
                Ok(EventSetMailbox {
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    default_ism: default_ism__.unwrap_or_default(),
                    default_hook: default_hook__.unwrap_or_default(),
                    new_owner: new_owner__.unwrap_or_default(),
                    renounce_ownership: renounce_ownership__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.EventSetMailbox", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for GenesisMailboxMessageWrapper {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.GenesisMailboxMessageWrapper", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("mailbox_id", ToString::to_string(&self.mailbox_id).as_str())?;
        }
        if true {
            struct_ser.serialize_field("message_id", &self.message_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for GenesisMailboxMessageWrapper {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "mailbox_id",
            "mailboxId",
            "message_id",
            "messageId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MailboxId,
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
                            "mailboxId" | "mailbox_id" => Ok(GeneratedField::MailboxId),
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
            type Value = GenesisMailboxMessageWrapper;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.GenesisMailboxMessageWrapper")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<GenesisMailboxMessageWrapper, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut mailbox_id__ = None;
                let mut message_id__ = None;
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
                        GeneratedField::MessageId => {
                            if message_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("messageId"));
                            }
                            message_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(GenesisMailboxMessageWrapper {
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                    message_id: message_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.GenesisMailboxMessageWrapper", FIELDS, GeneratedVisitor)
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
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.GenesisState", len)?;
        if let Some(v) = self.ism_genesis.as_ref() {
            struct_ser.serialize_field("ism_genesis", v)?;
        }
        if let Some(v) = self.post_dispatch_genesis.as_ref() {
            struct_ser.serialize_field("post_dispatch_genesis", v)?;
        }
        if true {
            struct_ser.serialize_field("mailboxes", &self.mailboxes)?;
        }
        if true {
            struct_ser.serialize_field("messages", &self.messages)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("ism_sequence", ToString::to_string(&self.ism_sequence).as_str())?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("post_dispatch_sequence", ToString::to_string(&self.post_dispatch_sequence).as_str())?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("app_sequence", ToString::to_string(&self.app_sequence).as_str())?;
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
            "ism_genesis",
            "ismGenesis",
            "post_dispatch_genesis",
            "postDispatchGenesis",
            "mailboxes",
            "messages",
            "ism_sequence",
            "ismSequence",
            "post_dispatch_sequence",
            "postDispatchSequence",
            "app_sequence",
            "appSequence",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IsmGenesis,
            PostDispatchGenesis,
            Mailboxes,
            Messages,
            IsmSequence,
            PostDispatchSequence,
            AppSequence,
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
                            "ismGenesis" | "ism_genesis" => Ok(GeneratedField::IsmGenesis),
                            "postDispatchGenesis" | "post_dispatch_genesis" => Ok(GeneratedField::PostDispatchGenesis),
                            "mailboxes" => Ok(GeneratedField::Mailboxes),
                            "messages" => Ok(GeneratedField::Messages),
                            "ismSequence" | "ism_sequence" => Ok(GeneratedField::IsmSequence),
                            "postDispatchSequence" | "post_dispatch_sequence" => Ok(GeneratedField::PostDispatchSequence),
                            "appSequence" | "app_sequence" => Ok(GeneratedField::AppSequence),
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
                formatter.write_str("struct hyperlane.core.v1.GenesisState")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<GenesisState, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism_genesis__ = None;
                let mut post_dispatch_genesis__ = None;
                let mut mailboxes__ = None;
                let mut messages__ = None;
                let mut ism_sequence__ = None;
                let mut post_dispatch_sequence__ = None;
                let mut app_sequence__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IsmGenesis => {
                            if ism_genesis__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismGenesis"));
                            }
                            ism_genesis__ = map_.next_value()?;
                        }
                        GeneratedField::PostDispatchGenesis => {
                            if post_dispatch_genesis__.is_some() {
                                return Err(serde::de::Error::duplicate_field("postDispatchGenesis"));
                            }
                            post_dispatch_genesis__ = map_.next_value()?;
                        }
                        GeneratedField::Mailboxes => {
                            if mailboxes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailboxes"));
                            }
                            mailboxes__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Messages => {
                            if messages__.is_some() {
                                return Err(serde::de::Error::duplicate_field("messages"));
                            }
                            messages__ = Some(map_.next_value()?);
                        }
                        GeneratedField::IsmSequence => {
                            if ism_sequence__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismSequence"));
                            }
                            ism_sequence__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::PostDispatchSequence => {
                            if post_dispatch_sequence__.is_some() {
                                return Err(serde::de::Error::duplicate_field("postDispatchSequence"));
                            }
                            post_dispatch_sequence__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::AppSequence => {
                            if app_sequence__.is_some() {
                                return Err(serde::de::Error::duplicate_field("appSequence"));
                            }
                            app_sequence__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(GenesisState {
                    ism_genesis: ism_genesis__,
                    post_dispatch_genesis: post_dispatch_genesis__,
                    mailboxes: mailboxes__.unwrap_or_default(),
                    messages: messages__.unwrap_or_default(),
                    ism_sequence: ism_sequence__.unwrap_or_default(),
                    post_dispatch_sequence: post_dispatch_sequence__.unwrap_or_default(),
                    app_sequence: app_sequence__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.GenesisState", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Mailbox {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.Mailbox", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("message_sent", &self.message_sent)?;
        }
        if true {
            struct_ser.serialize_field("message_received", &self.message_received)?;
        }
        if true {
            struct_ser.serialize_field("default_ism", &self.default_ism)?;
        }
        if true {
            struct_ser.serialize_field("default_hook", &self.default_hook)?;
        }
        if true {
            struct_ser.serialize_field("required_hook", &self.required_hook)?;
        }
        if true {
            struct_ser.serialize_field("local_domain", &self.local_domain)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Mailbox {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
            "owner",
            "message_sent",
            "messageSent",
            "message_received",
            "messageReceived",
            "default_ism",
            "defaultIsm",
            "default_hook",
            "defaultHook",
            "required_hook",
            "requiredHook",
            "local_domain",
            "localDomain",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
            Owner,
            MessageSent,
            MessageReceived,
            DefaultIsm,
            DefaultHook,
            RequiredHook,
            LocalDomain,
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
                            "messageSent" | "message_sent" => Ok(GeneratedField::MessageSent),
                            "messageReceived" | "message_received" => Ok(GeneratedField::MessageReceived),
                            "defaultIsm" | "default_ism" => Ok(GeneratedField::DefaultIsm),
                            "defaultHook" | "default_hook" => Ok(GeneratedField::DefaultHook),
                            "requiredHook" | "required_hook" => Ok(GeneratedField::RequiredHook),
                            "localDomain" | "local_domain" => Ok(GeneratedField::LocalDomain),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Mailbox;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.Mailbox")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Mailbox, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                let mut owner__ = None;
                let mut message_sent__ = None;
                let mut message_received__ = None;
                let mut default_ism__ = None;
                let mut default_hook__ = None;
                let mut required_hook__ = None;
                let mut local_domain__ = None;
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
                        GeneratedField::MessageSent => {
                            if message_sent__.is_some() {
                                return Err(serde::de::Error::duplicate_field("messageSent"));
                            }
                            message_sent__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::MessageReceived => {
                            if message_received__.is_some() {
                                return Err(serde::de::Error::duplicate_field("messageReceived"));
                            }
                            message_received__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::DefaultIsm => {
                            if default_ism__.is_some() {
                                return Err(serde::de::Error::duplicate_field("defaultIsm"));
                            }
                            default_ism__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DefaultHook => {
                            if default_hook__.is_some() {
                                return Err(serde::de::Error::duplicate_field("defaultHook"));
                            }
                            default_hook__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RequiredHook => {
                            if required_hook__.is_some() {
                                return Err(serde::de::Error::duplicate_field("requiredHook"));
                            }
                            required_hook__ = Some(map_.next_value()?);
                        }
                        GeneratedField::LocalDomain => {
                            if local_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("localDomain"));
                            }
                            local_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(Mailbox {
                    id: id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    message_sent: message_sent__.unwrap_or_default(),
                    message_received: message_received__.unwrap_or_default(),
                    default_ism: default_ism__.unwrap_or_default(),
                    default_hook: default_hook__.unwrap_or_default(),
                    required_hook: required_hook__.unwrap_or_default(),
                    local_domain: local_domain__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.Mailbox", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateMailbox {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.MsgCreateMailbox", len)?;
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("local_domain", &self.local_domain)?;
        }
        if true {
            struct_ser.serialize_field("default_ism", &self.default_ism)?;
        }
        if true {
            struct_ser.serialize_field("default_hook", &self.default_hook)?;
        }
        if true {
            struct_ser.serialize_field("required_hook", &self.required_hook)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateMailbox {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "owner",
            "local_domain",
            "localDomain",
            "default_ism",
            "defaultIsm",
            "default_hook",
            "defaultHook",
            "required_hook",
            "requiredHook",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Owner,
            LocalDomain,
            DefaultIsm,
            DefaultHook,
            RequiredHook,
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
                            "localDomain" | "local_domain" => Ok(GeneratedField::LocalDomain),
                            "defaultIsm" | "default_ism" => Ok(GeneratedField::DefaultIsm),
                            "defaultHook" | "default_hook" => Ok(GeneratedField::DefaultHook),
                            "requiredHook" | "required_hook" => Ok(GeneratedField::RequiredHook),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateMailbox;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.MsgCreateMailbox")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateMailbox, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut owner__ = None;
                let mut local_domain__ = None;
                let mut default_ism__ = None;
                let mut default_hook__ = None;
                let mut required_hook__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::LocalDomain => {
                            if local_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("localDomain"));
                            }
                            local_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::DefaultIsm => {
                            if default_ism__.is_some() {
                                return Err(serde::de::Error::duplicate_field("defaultIsm"));
                            }
                            default_ism__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DefaultHook => {
                            if default_hook__.is_some() {
                                return Err(serde::de::Error::duplicate_field("defaultHook"));
                            }
                            default_hook__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RequiredHook => {
                            if required_hook__.is_some() {
                                return Err(serde::de::Error::duplicate_field("requiredHook"));
                            }
                            required_hook__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgCreateMailbox {
                    owner: owner__.unwrap_or_default(),
                    local_domain: local_domain__.unwrap_or_default(),
                    default_ism: default_ism__.unwrap_or_default(),
                    default_hook: default_hook__.unwrap_or_default(),
                    required_hook: required_hook__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.MsgCreateMailbox", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateMailboxResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.MsgCreateMailboxResponse", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateMailboxResponse {
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
            type Value = MsgCreateMailboxResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.MsgCreateMailboxResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateMailboxResponse, V::Error>
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
                Ok(MsgCreateMailboxResponse {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.MsgCreateMailboxResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgProcessMessage {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.MsgProcessMessage", len)?;
        if true {
            struct_ser.serialize_field("mailbox_id", &self.mailbox_id)?;
        }
        if true {
            struct_ser.serialize_field("relayer", &self.relayer)?;
        }
        if true {
            struct_ser.serialize_field("metadata", &self.metadata)?;
        }
        if true {
            struct_ser.serialize_field("message", &self.message)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgProcessMessage {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "mailbox_id",
            "mailboxId",
            "relayer",
            "metadata",
            "message",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MailboxId,
            Relayer,
            Metadata,
            Message,
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
                            "relayer" => Ok(GeneratedField::Relayer),
                            "metadata" => Ok(GeneratedField::Metadata),
                            "message" => Ok(GeneratedField::Message),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgProcessMessage;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.MsgProcessMessage")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgProcessMessage, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut mailbox_id__ = None;
                let mut relayer__ = None;
                let mut metadata__ = None;
                let mut message__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::MailboxId => {
                            if mailbox_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailboxId"));
                            }
                            mailbox_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Relayer => {
                            if relayer__.is_some() {
                                return Err(serde::de::Error::duplicate_field("relayer"));
                            }
                            relayer__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Metadata => {
                            if metadata__.is_some() {
                                return Err(serde::de::Error::duplicate_field("metadata"));
                            }
                            metadata__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Message => {
                            if message__.is_some() {
                                return Err(serde::de::Error::duplicate_field("message"));
                            }
                            message__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgProcessMessage {
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                    relayer: relayer__.unwrap_or_default(),
                    metadata: metadata__.unwrap_or_default(),
                    message: message__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.MsgProcessMessage", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgProcessMessageResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.core.v1.MsgProcessMessageResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgProcessMessageResponse {
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
            type Value = MsgProcessMessageResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.MsgProcessMessageResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgProcessMessageResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(MsgProcessMessageResponse {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.MsgProcessMessageResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgSetMailbox {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.MsgSetMailbox", len)?;
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("mailbox_id", &self.mailbox_id)?;
        }
        if true {
            struct_ser.serialize_field("default_ism", &self.default_ism)?;
        }
        if true {
            struct_ser.serialize_field("default_hook", &self.default_hook)?;
        }
        if true {
            struct_ser.serialize_field("required_hook", &self.required_hook)?;
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
impl<'de> serde::Deserialize<'de> for MsgSetMailbox {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "owner",
            "mailbox_id",
            "mailboxId",
            "default_ism",
            "defaultIsm",
            "default_hook",
            "defaultHook",
            "required_hook",
            "requiredHook",
            "new_owner",
            "newOwner",
            "renounce_ownership",
            "renounceOwnership",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Owner,
            MailboxId,
            DefaultIsm,
            DefaultHook,
            RequiredHook,
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
                            "mailboxId" | "mailbox_id" => Ok(GeneratedField::MailboxId),
                            "defaultIsm" | "default_ism" => Ok(GeneratedField::DefaultIsm),
                            "defaultHook" | "default_hook" => Ok(GeneratedField::DefaultHook),
                            "requiredHook" | "required_hook" => Ok(GeneratedField::RequiredHook),
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
            type Value = MsgSetMailbox;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.MsgSetMailbox")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgSetMailbox, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut owner__ = None;
                let mut mailbox_id__ = None;
                let mut default_ism__ = None;
                let mut default_hook__ = None;
                let mut required_hook__ = None;
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
                        GeneratedField::MailboxId => {
                            if mailbox_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailboxId"));
                            }
                            mailbox_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DefaultIsm => {
                            if default_ism__.is_some() {
                                return Err(serde::de::Error::duplicate_field("defaultIsm"));
                            }
                            default_ism__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DefaultHook => {
                            if default_hook__.is_some() {
                                return Err(serde::de::Error::duplicate_field("defaultHook"));
                            }
                            default_hook__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RequiredHook => {
                            if required_hook__.is_some() {
                                return Err(serde::de::Error::duplicate_field("requiredHook"));
                            }
                            required_hook__ = Some(map_.next_value()?);
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
                Ok(MsgSetMailbox {
                    owner: owner__.unwrap_or_default(),
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                    default_ism: default_ism__.unwrap_or_default(),
                    default_hook: default_hook__.unwrap_or_default(),
                    required_hook: required_hook__.unwrap_or_default(),
                    new_owner: new_owner__.unwrap_or_default(),
                    renounce_ownership: renounce_ownership__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.MsgSetMailbox", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgSetMailboxResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.core.v1.MsgSetMailboxResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgSetMailboxResponse {
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
            type Value = MsgSetMailboxResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.MsgSetMailboxResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgSetMailboxResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(MsgSetMailboxResponse {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.MsgSetMailboxResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryDeliveredRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryDeliveredRequest", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if true {
            struct_ser.serialize_field("message_id", &self.message_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryDeliveredRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
            "message_id",
            "messageId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
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
                            "id" => Ok(GeneratedField::Id),
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
            type Value = QueryDeliveredRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryDeliveredRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryDeliveredRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                let mut message_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MessageId => {
                            if message_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("messageId"));
                            }
                            message_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryDeliveredRequest {
                    id: id__.unwrap_or_default(),
                    message_id: message_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryDeliveredRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryDeliveredResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryDeliveredResponse", len)?;
        if true {
            struct_ser.serialize_field("delivered", &self.delivered)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryDeliveredResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "delivered",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Delivered,
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
                            "delivered" => Ok(GeneratedField::Delivered),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryDeliveredResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryDeliveredResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryDeliveredResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut delivered__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Delivered => {
                            if delivered__.is_some() {
                                return Err(serde::de::Error::duplicate_field("delivered"));
                            }
                            delivered__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryDeliveredResponse {
                    delivered: delivered__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryDeliveredResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryMailboxRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryMailboxRequest", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryMailboxRequest {
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
            type Value = QueryMailboxRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryMailboxRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryMailboxRequest, V::Error>
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
                Ok(QueryMailboxRequest {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryMailboxRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryMailboxResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryMailboxResponse", len)?;
        if let Some(v) = self.mailbox.as_ref() {
            struct_ser.serialize_field("mailbox", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryMailboxResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "mailbox",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Mailbox,
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
                            "mailbox" => Ok(GeneratedField::Mailbox),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryMailboxResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryMailboxResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryMailboxResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut mailbox__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Mailbox => {
                            if mailbox__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailbox"));
                            }
                            mailbox__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryMailboxResponse {
                    mailbox: mailbox__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryMailboxResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryMailboxesRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryMailboxesRequest", len)?;
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryMailboxesRequest {
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
            type Value = QueryMailboxesRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryMailboxesRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryMailboxesRequest, V::Error>
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
                Ok(QueryMailboxesRequest {
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryMailboxesRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryMailboxesResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryMailboxesResponse", len)?;
        if true {
            struct_ser.serialize_field("mailboxes", &self.mailboxes)?;
        }
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryMailboxesResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "mailboxes",
            "pagination",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Mailboxes,
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
                            "mailboxes" => Ok(GeneratedField::Mailboxes),
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
            type Value = QueryMailboxesResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryMailboxesResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryMailboxesResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut mailboxes__ = None;
                let mut pagination__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Mailboxes => {
                            if mailboxes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailboxes"));
                            }
                            mailboxes__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Pagination => {
                            if pagination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pagination"));
                            }
                            pagination__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryMailboxesResponse {
                    mailboxes: mailboxes__.unwrap_or_default(),
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryMailboxesResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryRecipientIsmRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryRecipientIsmRequest", len)?;
        if true {
            struct_ser.serialize_field("recipient", &self.recipient)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryRecipientIsmRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "recipient",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Recipient,
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
                            "recipient" => Ok(GeneratedField::Recipient),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryRecipientIsmRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryRecipientIsmRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryRecipientIsmRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut recipient__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Recipient => {
                            if recipient__.is_some() {
                                return Err(serde::de::Error::duplicate_field("recipient"));
                            }
                            recipient__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryRecipientIsmRequest {
                    recipient: recipient__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryRecipientIsmRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryRecipientIsmResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryRecipientIsmResponse", len)?;
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryRecipientIsmResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism_id",
            "ismId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
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
            type Value = QueryRecipientIsmResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryRecipientIsmResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryRecipientIsmResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryRecipientIsmResponse {
                    ism_id: ism_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryRecipientIsmResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryRegisteredApps {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryRegisteredApps", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryRegisteredApps {
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
            type Value = QueryRegisteredApps;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryRegisteredApps")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryRegisteredApps, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(QueryRegisteredApps {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryRegisteredApps", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryRegisteredAppsResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryRegisteredAppsResponse", len)?;
        if true {
            struct_ser.serialize_field("ids", &self.ids)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryRegisteredAppsResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ids",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Ids,
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
                            "ids" => Ok(GeneratedField::Ids),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryRegisteredAppsResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryRegisteredAppsResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryRegisteredAppsResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ids__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Ids => {
                            if ids__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ids"));
                            }
                            ids__ = 
                                Some(map_.next_value::<Vec<::pbjson::private::NumberDeserialize<_>>>()?
                                    .into_iter().map(|x| x.0).collect())
                            ;
                        }
                    }
                }
                Ok(QueryRegisteredAppsResponse {
                    ids: ids__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryRegisteredAppsResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryRegisteredHooks {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryRegisteredHooks", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryRegisteredHooks {
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
            type Value = QueryRegisteredHooks;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryRegisteredHooks")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryRegisteredHooks, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(QueryRegisteredHooks {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryRegisteredHooks", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryRegisteredHooksResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryRegisteredHooksResponse", len)?;
        if true {
            struct_ser.serialize_field("ids", &self.ids)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryRegisteredHooksResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ids",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Ids,
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
                            "ids" => Ok(GeneratedField::Ids),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryRegisteredHooksResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryRegisteredHooksResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryRegisteredHooksResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ids__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Ids => {
                            if ids__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ids"));
                            }
                            ids__ = 
                                Some(map_.next_value::<Vec<::pbjson::private::NumberDeserialize<_>>>()?
                                    .into_iter().map(|x| x.0).collect())
                            ;
                        }
                    }
                }
                Ok(QueryRegisteredHooksResponse {
                    ids: ids__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryRegisteredHooksResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryRegisteredIsMs {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryRegisteredISMs", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryRegisteredIsMs {
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
            type Value = QueryRegisteredIsMs;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryRegisteredISMs")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryRegisteredIsMs, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(QueryRegisteredIsMs {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryRegisteredISMs", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryRegisteredIsMsResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryRegisteredISMsResponse", len)?;
        if true {
            struct_ser.serialize_field("ids", &self.ids)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryRegisteredIsMsResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ids",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Ids,
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
                            "ids" => Ok(GeneratedField::Ids),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryRegisteredIsMsResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryRegisteredISMsResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryRegisteredIsMsResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ids__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Ids => {
                            if ids__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ids"));
                            }
                            ids__ = 
                                Some(map_.next_value::<Vec<::pbjson::private::NumberDeserialize<_>>>()?
                                    .into_iter().map(|x| x.0).collect())
                            ;
                        }
                    }
                }
                Ok(QueryRegisteredIsMsResponse {
                    ids: ids__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryRegisteredISMsResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryVerifyDryRunRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryVerifyDryRunRequest", len)?;
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
        }
        if true {
            struct_ser.serialize_field("message", &self.message)?;
        }
        if true {
            struct_ser.serialize_field("metadata", &self.metadata)?;
        }
        if true {
            struct_ser.serialize_field("gas_limit", &self.gas_limit)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryVerifyDryRunRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ism_id",
            "ismId",
            "message",
            "metadata",
            "gas_limit",
            "gasLimit",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IsmId,
            Message,
            Metadata,
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
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
                            "message" => Ok(GeneratedField::Message),
                            "metadata" => Ok(GeneratedField::Metadata),
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
            type Value = QueryVerifyDryRunRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryVerifyDryRunRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryVerifyDryRunRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ism_id__ = None;
                let mut message__ = None;
                let mut metadata__ = None;
                let mut gas_limit__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Message => {
                            if message__.is_some() {
                                return Err(serde::de::Error::duplicate_field("message"));
                            }
                            message__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Metadata => {
                            if metadata__.is_some() {
                                return Err(serde::de::Error::duplicate_field("metadata"));
                            }
                            metadata__ = Some(map_.next_value()?);
                        }
                        GeneratedField::GasLimit => {
                            if gas_limit__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gasLimit"));
                            }
                            gas_limit__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryVerifyDryRunRequest {
                    ism_id: ism_id__.unwrap_or_default(),
                    message: message__.unwrap_or_default(),
                    metadata: metadata__.unwrap_or_default(),
                    gas_limit: gas_limit__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryVerifyDryRunRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryVerifyDryRunResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.v1.QueryVerifyDryRunResponse", len)?;
        if true {
            struct_ser.serialize_field("verified", &self.verified)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryVerifyDryRunResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "verified",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Verified,
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
                            "verified" => Ok(GeneratedField::Verified),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryVerifyDryRunResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.v1.QueryVerifyDryRunResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryVerifyDryRunResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut verified__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Verified => {
                            if verified__.is_some() {
                                return Err(serde::de::Error::duplicate_field("verified"));
                            }
                            verified__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryVerifyDryRunResponse {
                    verified: verified__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.v1.QueryVerifyDryRunResponse", FIELDS, GeneratedVisitor)
    }
}
