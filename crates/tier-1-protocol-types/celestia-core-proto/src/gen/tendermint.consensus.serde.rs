impl serde::Serialize for BlockPart {
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
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.BlockPart", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("height", ToString::to_string(&self.height).as_str())?;
        }
        if true {
            struct_ser.serialize_field("round", &self.round)?;
        }
        if let Some(v) = self.part.as_ref() {
            struct_ser.serialize_field("part", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlockPart {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "height",
            "round",
            "part",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Height,
            Round,
            Part,
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
                            "height" => Ok(GeneratedField::Height),
                            "round" => Ok(GeneratedField::Round),
                            "part" => Ok(GeneratedField::Part),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlockPart;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.BlockPart")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlockPart, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut height__ = None;
                let mut round__ = None;
                let mut part__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Height => {
                            if height__.is_some() {
                                return Err(serde::de::Error::duplicate_field("height"));
                            }
                            height__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Round => {
                            if round__.is_some() {
                                return Err(serde::de::Error::duplicate_field("round"));
                            }
                            round__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Part => {
                            if part__.is_some() {
                                return Err(serde::de::Error::duplicate_field("part"));
                            }
                            part__ = map_.next_value()?;
                        }
                    }
                }
                Ok(BlockPart {
                    height: height__.unwrap_or_default(),
                    round: round__.unwrap_or_default(),
                    part: part__,
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.BlockPart", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EndHeight {
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
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.EndHeight", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("height", ToString::to_string(&self.height).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EndHeight {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "height",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Height,
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
                            "height" => Ok(GeneratedField::Height),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EndHeight;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.EndHeight")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EndHeight, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut height__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Height => {
                            if height__.is_some() {
                                return Err(serde::de::Error::duplicate_field("height"));
                            }
                            height__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(EndHeight {
                    height: height__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.EndHeight", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for HasVote {
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
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.HasVote", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("height", ToString::to_string(&self.height).as_str())?;
        }
        if true {
            struct_ser.serialize_field("round", &self.round)?;
        }
        if true {
            let v = super::types::SignedMsgType::try_from(self.r#type)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.r#type)))?;
            struct_ser.serialize_field("type", &v)?;
        }
        if true {
            struct_ser.serialize_field("index", &self.index)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for HasVote {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "height",
            "round",
            "type",
            "index",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Height,
            Round,
            Type,
            Index,
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
                            "height" => Ok(GeneratedField::Height),
                            "round" => Ok(GeneratedField::Round),
                            "type" => Ok(GeneratedField::Type),
                            "index" => Ok(GeneratedField::Index),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = HasVote;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.HasVote")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<HasVote, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut height__ = None;
                let mut round__ = None;
                let mut r#type__ = None;
                let mut index__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Height => {
                            if height__.is_some() {
                                return Err(serde::de::Error::duplicate_field("height"));
                            }
                            height__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Round => {
                            if round__.is_some() {
                                return Err(serde::de::Error::duplicate_field("round"));
                            }
                            round__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Type => {
                            if r#type__.is_some() {
                                return Err(serde::de::Error::duplicate_field("type"));
                            }
                            r#type__ = Some(map_.next_value::<super::types::SignedMsgType>()? as i32);
                        }
                        GeneratedField::Index => {
                            if index__.is_some() {
                                return Err(serde::de::Error::duplicate_field("index"));
                            }
                            index__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(HasVote {
                    height: height__.unwrap_or_default(),
                    round: round__.unwrap_or_default(),
                    r#type: r#type__.unwrap_or_default(),
                    index: index__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.HasVote", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Message {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.sum.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.Message", len)?;
        if let Some(v) = self.sum.as_ref() {
            match v {
                message::Sum::NewRoundStep(v) => {
                    struct_ser.serialize_field("new_round_step", v)?;
                }
                message::Sum::NewValidBlock(v) => {
                    struct_ser.serialize_field("new_valid_block", v)?;
                }
                message::Sum::Proposal(v) => {
                    struct_ser.serialize_field("proposal", v)?;
                }
                message::Sum::ProposalPol(v) => {
                    struct_ser.serialize_field("proposal_pol", v)?;
                }
                message::Sum::BlockPart(v) => {
                    struct_ser.serialize_field("block_part", v)?;
                }
                message::Sum::Vote(v) => {
                    struct_ser.serialize_field("vote", v)?;
                }
                message::Sum::HasVote(v) => {
                    struct_ser.serialize_field("has_vote", v)?;
                }
                message::Sum::VoteSetMaj23(v) => {
                    struct_ser.serialize_field("vote_set_maj23", v)?;
                }
                message::Sum::VoteSetBits(v) => {
                    struct_ser.serialize_field("vote_set_bits", v)?;
                }
            }
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Message {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "new_round_step",
            "newRoundStep",
            "new_valid_block",
            "newValidBlock",
            "proposal",
            "proposal_pol",
            "proposalPol",
            "block_part",
            "blockPart",
            "vote",
            "has_vote",
            "hasVote",
            "vote_set_maj23",
            "voteSetMaj23",
            "vote_set_bits",
            "voteSetBits",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            NewRoundStep,
            NewValidBlock,
            Proposal,
            ProposalPol,
            BlockPart,
            Vote,
            HasVote,
            VoteSetMaj23,
            VoteSetBits,
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
                            "newRoundStep" | "new_round_step" => Ok(GeneratedField::NewRoundStep),
                            "newValidBlock" | "new_valid_block" => Ok(GeneratedField::NewValidBlock),
                            "proposal" => Ok(GeneratedField::Proposal),
                            "proposalPol" | "proposal_pol" => Ok(GeneratedField::ProposalPol),
                            "blockPart" | "block_part" => Ok(GeneratedField::BlockPart),
                            "vote" => Ok(GeneratedField::Vote),
                            "hasVote" | "has_vote" => Ok(GeneratedField::HasVote),
                            "voteSetMaj23" | "vote_set_maj23" => Ok(GeneratedField::VoteSetMaj23),
                            "voteSetBits" | "vote_set_bits" => Ok(GeneratedField::VoteSetBits),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Message;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.Message")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Message, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut sum__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::NewRoundStep => {
                            if sum__.is_some() {
                                return Err(serde::de::Error::duplicate_field("newRoundStep"));
                            }
                            sum__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Sum::NewRoundStep)
;
                        }
                        GeneratedField::NewValidBlock => {
                            if sum__.is_some() {
                                return Err(serde::de::Error::duplicate_field("newValidBlock"));
                            }
                            sum__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Sum::NewValidBlock)
;
                        }
                        GeneratedField::Proposal => {
                            if sum__.is_some() {
                                return Err(serde::de::Error::duplicate_field("proposal"));
                            }
                            sum__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Sum::Proposal)
;
                        }
                        GeneratedField::ProposalPol => {
                            if sum__.is_some() {
                                return Err(serde::de::Error::duplicate_field("proposalPol"));
                            }
                            sum__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Sum::ProposalPol)
;
                        }
                        GeneratedField::BlockPart => {
                            if sum__.is_some() {
                                return Err(serde::de::Error::duplicate_field("blockPart"));
                            }
                            sum__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Sum::BlockPart)
;
                        }
                        GeneratedField::Vote => {
                            if sum__.is_some() {
                                return Err(serde::de::Error::duplicate_field("vote"));
                            }
                            sum__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Sum::Vote)
;
                        }
                        GeneratedField::HasVote => {
                            if sum__.is_some() {
                                return Err(serde::de::Error::duplicate_field("hasVote"));
                            }
                            sum__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Sum::HasVote)
;
                        }
                        GeneratedField::VoteSetMaj23 => {
                            if sum__.is_some() {
                                return Err(serde::de::Error::duplicate_field("voteSetMaj23"));
                            }
                            sum__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Sum::VoteSetMaj23)
;
                        }
                        GeneratedField::VoteSetBits => {
                            if sum__.is_some() {
                                return Err(serde::de::Error::duplicate_field("voteSetBits"));
                            }
                            sum__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Sum::VoteSetBits)
;
                        }
                    }
                }
                Ok(Message {
                    sum: sum__,
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.Message", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgInfo {
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
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.MsgInfo", len)?;
        if let Some(v) = self.msg.as_ref() {
            struct_ser.serialize_field("msg", v)?;
        }
        if true {
            struct_ser.serialize_field("peer_id", &self.peer_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgInfo {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "msg",
            "peer_id",
            "peerId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Msg,
            PeerId,
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
                            "msg" => Ok(GeneratedField::Msg),
                            "peerId" | "peer_id" => Ok(GeneratedField::PeerId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgInfo;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.MsgInfo")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgInfo, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut msg__ = None;
                let mut peer_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Msg => {
                            if msg__.is_some() {
                                return Err(serde::de::Error::duplicate_field("msg"));
                            }
                            msg__ = map_.next_value()?;
                        }
                        GeneratedField::PeerId => {
                            if peer_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("peerId"));
                            }
                            peer_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgInfo {
                    msg: msg__,
                    peer_id: peer_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.MsgInfo", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for NewRoundStep {
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
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.NewRoundStep", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("height", ToString::to_string(&self.height).as_str())?;
        }
        if true {
            struct_ser.serialize_field("round", &self.round)?;
        }
        if true {
            struct_ser.serialize_field("step", &self.step)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("seconds_since_start_time", ToString::to_string(&self.seconds_since_start_time).as_str())?;
        }
        if true {
            struct_ser.serialize_field("last_commit_round", &self.last_commit_round)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for NewRoundStep {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "height",
            "round",
            "step",
            "seconds_since_start_time",
            "secondsSinceStartTime",
            "last_commit_round",
            "lastCommitRound",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Height,
            Round,
            Step,
            SecondsSinceStartTime,
            LastCommitRound,
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
                            "height" => Ok(GeneratedField::Height),
                            "round" => Ok(GeneratedField::Round),
                            "step" => Ok(GeneratedField::Step),
                            "secondsSinceStartTime" | "seconds_since_start_time" => Ok(GeneratedField::SecondsSinceStartTime),
                            "lastCommitRound" | "last_commit_round" => Ok(GeneratedField::LastCommitRound),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = NewRoundStep;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.NewRoundStep")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<NewRoundStep, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut height__ = None;
                let mut round__ = None;
                let mut step__ = None;
                let mut seconds_since_start_time__ = None;
                let mut last_commit_round__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Height => {
                            if height__.is_some() {
                                return Err(serde::de::Error::duplicate_field("height"));
                            }
                            height__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Round => {
                            if round__.is_some() {
                                return Err(serde::de::Error::duplicate_field("round"));
                            }
                            round__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Step => {
                            if step__.is_some() {
                                return Err(serde::de::Error::duplicate_field("step"));
                            }
                            step__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::SecondsSinceStartTime => {
                            if seconds_since_start_time__.is_some() {
                                return Err(serde::de::Error::duplicate_field("secondsSinceStartTime"));
                            }
                            seconds_since_start_time__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::LastCommitRound => {
                            if last_commit_round__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastCommitRound"));
                            }
                            last_commit_round__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(NewRoundStep {
                    height: height__.unwrap_or_default(),
                    round: round__.unwrap_or_default(),
                    step: step__.unwrap_or_default(),
                    seconds_since_start_time: seconds_since_start_time__.unwrap_or_default(),
                    last_commit_round: last_commit_round__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.NewRoundStep", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for NewValidBlock {
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
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.NewValidBlock", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("height", ToString::to_string(&self.height).as_str())?;
        }
        if true {
            struct_ser.serialize_field("round", &self.round)?;
        }
        if let Some(v) = self.block_part_set_header.as_ref() {
            struct_ser.serialize_field("block_part_set_header", v)?;
        }
        if let Some(v) = self.block_parts.as_ref() {
            struct_ser.serialize_field("block_parts", v)?;
        }
        if true {
            struct_ser.serialize_field("is_commit", &self.is_commit)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for NewValidBlock {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "height",
            "round",
            "block_part_set_header",
            "blockPartSetHeader",
            "block_parts",
            "blockParts",
            "is_commit",
            "isCommit",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Height,
            Round,
            BlockPartSetHeader,
            BlockParts,
            IsCommit,
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
                            "height" => Ok(GeneratedField::Height),
                            "round" => Ok(GeneratedField::Round),
                            "blockPartSetHeader" | "block_part_set_header" => Ok(GeneratedField::BlockPartSetHeader),
                            "blockParts" | "block_parts" => Ok(GeneratedField::BlockParts),
                            "isCommit" | "is_commit" => Ok(GeneratedField::IsCommit),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = NewValidBlock;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.NewValidBlock")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<NewValidBlock, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut height__ = None;
                let mut round__ = None;
                let mut block_part_set_header__ = None;
                let mut block_parts__ = None;
                let mut is_commit__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Height => {
                            if height__.is_some() {
                                return Err(serde::de::Error::duplicate_field("height"));
                            }
                            height__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Round => {
                            if round__.is_some() {
                                return Err(serde::de::Error::duplicate_field("round"));
                            }
                            round__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::BlockPartSetHeader => {
                            if block_part_set_header__.is_some() {
                                return Err(serde::de::Error::duplicate_field("blockPartSetHeader"));
                            }
                            block_part_set_header__ = map_.next_value()?;
                        }
                        GeneratedField::BlockParts => {
                            if block_parts__.is_some() {
                                return Err(serde::de::Error::duplicate_field("blockParts"));
                            }
                            block_parts__ = map_.next_value()?;
                        }
                        GeneratedField::IsCommit => {
                            if is_commit__.is_some() {
                                return Err(serde::de::Error::duplicate_field("isCommit"));
                            }
                            is_commit__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(NewValidBlock {
                    height: height__.unwrap_or_default(),
                    round: round__.unwrap_or_default(),
                    block_part_set_header: block_part_set_header__,
                    block_parts: block_parts__,
                    is_commit: is_commit__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.NewValidBlock", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Proposal {
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
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.Proposal", len)?;
        if let Some(v) = self.proposal.as_ref() {
            struct_ser.serialize_field("proposal", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Proposal {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "proposal",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Proposal,
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
                            "proposal" => Ok(GeneratedField::Proposal),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Proposal;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.Proposal")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Proposal, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut proposal__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Proposal => {
                            if proposal__.is_some() {
                                return Err(serde::de::Error::duplicate_field("proposal"));
                            }
                            proposal__ = map_.next_value()?;
                        }
                    }
                }
                Ok(Proposal {
                    proposal: proposal__,
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.Proposal", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ProposalPol {
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
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.ProposalPOL", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("height", ToString::to_string(&self.height).as_str())?;
        }
        if true {
            struct_ser.serialize_field("proposal_pol_round", &self.proposal_pol_round)?;
        }
        if let Some(v) = self.proposal_pol.as_ref() {
            struct_ser.serialize_field("proposal_pol", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ProposalPol {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "height",
            "proposal_pol_round",
            "proposalPolRound",
            "proposal_pol",
            "proposalPol",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Height,
            ProposalPolRound,
            ProposalPol,
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
                            "height" => Ok(GeneratedField::Height),
                            "proposalPolRound" | "proposal_pol_round" => Ok(GeneratedField::ProposalPolRound),
                            "proposalPol" | "proposal_pol" => Ok(GeneratedField::ProposalPol),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ProposalPol;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.ProposalPOL")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ProposalPol, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut height__ = None;
                let mut proposal_pol_round__ = None;
                let mut proposal_pol__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Height => {
                            if height__.is_some() {
                                return Err(serde::de::Error::duplicate_field("height"));
                            }
                            height__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ProposalPolRound => {
                            if proposal_pol_round__.is_some() {
                                return Err(serde::de::Error::duplicate_field("proposalPolRound"));
                            }
                            proposal_pol_round__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ProposalPol => {
                            if proposal_pol__.is_some() {
                                return Err(serde::de::Error::duplicate_field("proposalPol"));
                            }
                            proposal_pol__ = map_.next_value()?;
                        }
                    }
                }
                Ok(ProposalPol {
                    height: height__.unwrap_or_default(),
                    proposal_pol_round: proposal_pol_round__.unwrap_or_default(),
                    proposal_pol: proposal_pol__,
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.ProposalPOL", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for TimedWalMessage {
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
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.TimedWALMessage", len)?;
        if let Some(v) = self.time.as_ref() {
            struct_ser.serialize_field("time", v)?;
        }
        if let Some(v) = self.msg.as_ref() {
            struct_ser.serialize_field("msg", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for TimedWalMessage {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "time",
            "msg",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Time,
            Msg,
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
                            "time" => Ok(GeneratedField::Time),
                            "msg" => Ok(GeneratedField::Msg),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = TimedWalMessage;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.TimedWALMessage")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<TimedWalMessage, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut time__ = None;
                let mut msg__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Time => {
                            if time__.is_some() {
                                return Err(serde::de::Error::duplicate_field("time"));
                            }
                            time__ = map_.next_value()?;
                        }
                        GeneratedField::Msg => {
                            if msg__.is_some() {
                                return Err(serde::de::Error::duplicate_field("msg"));
                            }
                            msg__ = map_.next_value()?;
                        }
                    }
                }
                Ok(TimedWalMessage {
                    time: time__,
                    msg: msg__,
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.TimedWALMessage", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.TimeoutInfo", len)?;
        if let Some(v) = self.duration.as_ref() {
            struct_ser.serialize_field("duration", v)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("height", ToString::to_string(&self.height).as_str())?;
        }
        if true {
            struct_ser.serialize_field("round", &self.round)?;
        }
        if true {
            struct_ser.serialize_field("step", &self.step)?;
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
            "duration",
            "height",
            "round",
            "step",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Duration,
            Height,
            Round,
            Step,
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
                            "duration" => Ok(GeneratedField::Duration),
                            "height" => Ok(GeneratedField::Height),
                            "round" => Ok(GeneratedField::Round),
                            "step" => Ok(GeneratedField::Step),
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
                formatter.write_str("struct tendermint.consensus.TimeoutInfo")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<TimeoutInfo, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut duration__ = None;
                let mut height__ = None;
                let mut round__ = None;
                let mut step__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Duration => {
                            if duration__.is_some() {
                                return Err(serde::de::Error::duplicate_field("duration"));
                            }
                            duration__ = map_.next_value()?;
                        }
                        GeneratedField::Height => {
                            if height__.is_some() {
                                return Err(serde::de::Error::duplicate_field("height"));
                            }
                            height__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Round => {
                            if round__.is_some() {
                                return Err(serde::de::Error::duplicate_field("round"));
                            }
                            round__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Step => {
                            if step__.is_some() {
                                return Err(serde::de::Error::duplicate_field("step"));
                            }
                            step__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(TimeoutInfo {
                    duration: duration__,
                    height: height__.unwrap_or_default(),
                    round: round__.unwrap_or_default(),
                    step: step__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.TimeoutInfo", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Vote {
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
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.Vote", len)?;
        if let Some(v) = self.vote.as_ref() {
            struct_ser.serialize_field("vote", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Vote {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "vote",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Vote,
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
                            "vote" => Ok(GeneratedField::Vote),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Vote;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.Vote")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Vote, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut vote__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Vote => {
                            if vote__.is_some() {
                                return Err(serde::de::Error::duplicate_field("vote"));
                            }
                            vote__ = map_.next_value()?;
                        }
                    }
                }
                Ok(Vote {
                    vote: vote__,
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.Vote", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for VoteSetBits {
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
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.VoteSetBits", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("height", ToString::to_string(&self.height).as_str())?;
        }
        if true {
            struct_ser.serialize_field("round", &self.round)?;
        }
        if true {
            let v = super::types::SignedMsgType::try_from(self.r#type)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.r#type)))?;
            struct_ser.serialize_field("type", &v)?;
        }
        if let Some(v) = self.block_id.as_ref() {
            struct_ser.serialize_field("block_id", v)?;
        }
        if let Some(v) = self.votes.as_ref() {
            struct_ser.serialize_field("votes", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for VoteSetBits {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "height",
            "round",
            "type",
            "block_id",
            "blockId",
            "votes",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Height,
            Round,
            Type,
            BlockId,
            Votes,
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
                            "height" => Ok(GeneratedField::Height),
                            "round" => Ok(GeneratedField::Round),
                            "type" => Ok(GeneratedField::Type),
                            "blockId" | "block_id" => Ok(GeneratedField::BlockId),
                            "votes" => Ok(GeneratedField::Votes),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = VoteSetBits;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.VoteSetBits")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<VoteSetBits, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut height__ = None;
                let mut round__ = None;
                let mut r#type__ = None;
                let mut block_id__ = None;
                let mut votes__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Height => {
                            if height__.is_some() {
                                return Err(serde::de::Error::duplicate_field("height"));
                            }
                            height__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Round => {
                            if round__.is_some() {
                                return Err(serde::de::Error::duplicate_field("round"));
                            }
                            round__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Type => {
                            if r#type__.is_some() {
                                return Err(serde::de::Error::duplicate_field("type"));
                            }
                            r#type__ = Some(map_.next_value::<super::types::SignedMsgType>()? as i32);
                        }
                        GeneratedField::BlockId => {
                            if block_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("blockId"));
                            }
                            block_id__ = map_.next_value()?;
                        }
                        GeneratedField::Votes => {
                            if votes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("votes"));
                            }
                            votes__ = map_.next_value()?;
                        }
                    }
                }
                Ok(VoteSetBits {
                    height: height__.unwrap_or_default(),
                    round: round__.unwrap_or_default(),
                    r#type: r#type__.unwrap_or_default(),
                    block_id: block_id__,
                    votes: votes__,
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.VoteSetBits", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for VoteSetMaj23 {
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
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.VoteSetMaj23", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("height", ToString::to_string(&self.height).as_str())?;
        }
        if true {
            struct_ser.serialize_field("round", &self.round)?;
        }
        if true {
            let v = super::types::SignedMsgType::try_from(self.r#type)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.r#type)))?;
            struct_ser.serialize_field("type", &v)?;
        }
        if let Some(v) = self.block_id.as_ref() {
            struct_ser.serialize_field("block_id", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for VoteSetMaj23 {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "height",
            "round",
            "type",
            "block_id",
            "blockId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Height,
            Round,
            Type,
            BlockId,
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
                            "height" => Ok(GeneratedField::Height),
                            "round" => Ok(GeneratedField::Round),
                            "type" => Ok(GeneratedField::Type),
                            "blockId" | "block_id" => Ok(GeneratedField::BlockId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = VoteSetMaj23;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.VoteSetMaj23")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<VoteSetMaj23, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut height__ = None;
                let mut round__ = None;
                let mut r#type__ = None;
                let mut block_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Height => {
                            if height__.is_some() {
                                return Err(serde::de::Error::duplicate_field("height"));
                            }
                            height__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Round => {
                            if round__.is_some() {
                                return Err(serde::de::Error::duplicate_field("round"));
                            }
                            round__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Type => {
                            if r#type__.is_some() {
                                return Err(serde::de::Error::duplicate_field("type"));
                            }
                            r#type__ = Some(map_.next_value::<super::types::SignedMsgType>()? as i32);
                        }
                        GeneratedField::BlockId => {
                            if block_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("blockId"));
                            }
                            block_id__ = map_.next_value()?;
                        }
                    }
                }
                Ok(VoteSetMaj23 {
                    height: height__.unwrap_or_default(),
                    round: round__.unwrap_or_default(),
                    r#type: r#type__.unwrap_or_default(),
                    block_id: block_id__,
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.VoteSetMaj23", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for WalMessage {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.sum.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("tendermint.consensus.WALMessage", len)?;
        if let Some(v) = self.sum.as_ref() {
            match v {
                wal_message::Sum::EventDataRoundState(v) => {
                    struct_ser.serialize_field("event_data_round_state", v)?;
                }
                wal_message::Sum::MsgInfo(v) => {
                    struct_ser.serialize_field("msg_info", v)?;
                }
                wal_message::Sum::TimeoutInfo(v) => {
                    struct_ser.serialize_field("timeout_info", v)?;
                }
                wal_message::Sum::EndHeight(v) => {
                    struct_ser.serialize_field("end_height", v)?;
                }
            }
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for WalMessage {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "event_data_round_state",
            "eventDataRoundState",
            "msg_info",
            "msgInfo",
            "timeout_info",
            "timeoutInfo",
            "end_height",
            "endHeight",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            EventDataRoundState,
            MsgInfo,
            TimeoutInfo,
            EndHeight,
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
                            "eventDataRoundState" | "event_data_round_state" => Ok(GeneratedField::EventDataRoundState),
                            "msgInfo" | "msg_info" => Ok(GeneratedField::MsgInfo),
                            "timeoutInfo" | "timeout_info" => Ok(GeneratedField::TimeoutInfo),
                            "endHeight" | "end_height" => Ok(GeneratedField::EndHeight),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = WalMessage;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.consensus.WALMessage")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<WalMessage, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut sum__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::EventDataRoundState => {
                            if sum__.is_some() {
                                return Err(serde::de::Error::duplicate_field("eventDataRoundState"));
                            }
                            sum__ = map_.next_value::<::std::option::Option<_>>()?.map(wal_message::Sum::EventDataRoundState)
;
                        }
                        GeneratedField::MsgInfo => {
                            if sum__.is_some() {
                                return Err(serde::de::Error::duplicate_field("msgInfo"));
                            }
                            sum__ = map_.next_value::<::std::option::Option<_>>()?.map(wal_message::Sum::MsgInfo)
;
                        }
                        GeneratedField::TimeoutInfo => {
                            if sum__.is_some() {
                                return Err(serde::de::Error::duplicate_field("timeoutInfo"));
                            }
                            sum__ = map_.next_value::<::std::option::Option<_>>()?.map(wal_message::Sum::TimeoutInfo)
;
                        }
                        GeneratedField::EndHeight => {
                            if sum__.is_some() {
                                return Err(serde::de::Error::duplicate_field("endHeight"));
                            }
                            sum__ = map_.next_value::<::std::option::Option<_>>()?.map(wal_message::Sum::EndHeight)
;
                        }
                    }
                }
                Ok(WalMessage {
                    sum: sum__,
                })
            }
        }
        deserializer.deserialize_struct("tendermint.consensus.WALMessage", FIELDS, GeneratedVisitor)
    }
}
