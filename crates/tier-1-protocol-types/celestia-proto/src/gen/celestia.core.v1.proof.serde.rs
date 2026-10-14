impl serde::Serialize for NmtProof {
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
        let mut struct_ser = serializer.serialize_struct("celestia.core.v1.proof.NMTProof", len)?;
        if true {
            struct_ser.serialize_field("start", &self.start)?;
        }
        if true {
            struct_ser.serialize_field("end", &self.end)?;
        }
        if true {
            struct_ser.serialize_field("nodes", &self.nodes.iter().map(pbjson::private::base64::encode).collect::<Vec<_>>())?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("leaf_hash", pbjson::private::base64::encode(&self.leaf_hash).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for NmtProof {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "start",
            "end",
            "nodes",
            "leaf_hash",
            "leafHash",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Start,
            End,
            Nodes,
            LeafHash,
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
                            "start" => Ok(GeneratedField::Start),
                            "end" => Ok(GeneratedField::End),
                            "nodes" => Ok(GeneratedField::Nodes),
                            "leafHash" | "leaf_hash" => Ok(GeneratedField::LeafHash),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = NmtProof;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.core.v1.proof.NMTProof")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<NmtProof, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut start__ = None;
                let mut end__ = None;
                let mut nodes__ = None;
                let mut leaf_hash__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Start => {
                            if start__.is_some() {
                                return Err(serde::de::Error::duplicate_field("start"));
                            }
                            start__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::End => {
                            if end__.is_some() {
                                return Err(serde::de::Error::duplicate_field("end"));
                            }
                            end__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Nodes => {
                            if nodes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("nodes"));
                            }
                            nodes__ = 
                                Some(map_.next_value::<Vec<::pbjson::private::BytesDeserialize<_>>>()?
                                    .into_iter().map(|x| x.0).collect())
                            ;
                        }
                        GeneratedField::LeafHash => {
                            if leaf_hash__.is_some() {
                                return Err(serde::de::Error::duplicate_field("leafHash"));
                            }
                            leaf_hash__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(NmtProof {
                    start: start__.unwrap_or_default(),
                    end: end__.unwrap_or_default(),
                    nodes: nodes__.unwrap_or_default(),
                    leaf_hash: leaf_hash__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.core.v1.proof.NMTProof", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Proof {
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
        let mut struct_ser = serializer.serialize_struct("celestia.core.v1.proof.Proof", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("total", ToString::to_string(&self.total).as_str())?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("index", ToString::to_string(&self.index).as_str())?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("leaf_hash", pbjson::private::base64::encode(&self.leaf_hash).as_str())?;
        }
        if true {
            struct_ser.serialize_field("aunts", &self.aunts.iter().map(pbjson::private::base64::encode).collect::<Vec<_>>())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Proof {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "total",
            "index",
            "leaf_hash",
            "leafHash",
            "aunts",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Total,
            Index,
            LeafHash,
            Aunts,
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
                            "total" => Ok(GeneratedField::Total),
                            "index" => Ok(GeneratedField::Index),
                            "leafHash" | "leaf_hash" => Ok(GeneratedField::LeafHash),
                            "aunts" => Ok(GeneratedField::Aunts),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Proof;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.core.v1.proof.Proof")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Proof, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut total__ = None;
                let mut index__ = None;
                let mut leaf_hash__ = None;
                let mut aunts__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Total => {
                            if total__.is_some() {
                                return Err(serde::de::Error::duplicate_field("total"));
                            }
                            total__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Index => {
                            if index__.is_some() {
                                return Err(serde::de::Error::duplicate_field("index"));
                            }
                            index__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::LeafHash => {
                            if leaf_hash__.is_some() {
                                return Err(serde::de::Error::duplicate_field("leafHash"));
                            }
                            leaf_hash__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Aunts => {
                            if aunts__.is_some() {
                                return Err(serde::de::Error::duplicate_field("aunts"));
                            }
                            aunts__ = 
                                Some(map_.next_value::<Vec<::pbjson::private::BytesDeserialize<_>>>()?
                                    .into_iter().map(|x| x.0).collect())
                            ;
                        }
                    }
                }
                Ok(Proof {
                    total: total__.unwrap_or_default(),
                    index: index__.unwrap_or_default(),
                    leaf_hash: leaf_hash__.unwrap_or_default(),
                    aunts: aunts__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.core.v1.proof.Proof", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for RowProof {
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
        let mut struct_ser = serializer.serialize_struct("celestia.core.v1.proof.RowProof", len)?;
        if true {
            struct_ser.serialize_field("row_roots", &self.row_roots.iter().map(pbjson::private::base64::encode).collect::<Vec<_>>())?;
        }
        if true {
            struct_ser.serialize_field("proofs", &self.proofs)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("root", pbjson::private::base64::encode(&self.root).as_str())?;
        }
        if true {
            struct_ser.serialize_field("start_row", &self.start_row)?;
        }
        if true {
            struct_ser.serialize_field("end_row", &self.end_row)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for RowProof {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "row_roots",
            "rowRoots",
            "proofs",
            "root",
            "start_row",
            "startRow",
            "end_row",
            "endRow",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            RowRoots,
            Proofs,
            Root,
            StartRow,
            EndRow,
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
                            "rowRoots" | "row_roots" => Ok(GeneratedField::RowRoots),
                            "proofs" => Ok(GeneratedField::Proofs),
                            "root" => Ok(GeneratedField::Root),
                            "startRow" | "start_row" => Ok(GeneratedField::StartRow),
                            "endRow" | "end_row" => Ok(GeneratedField::EndRow),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = RowProof;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.core.v1.proof.RowProof")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<RowProof, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut row_roots__ = None;
                let mut proofs__ = None;
                let mut root__ = None;
                let mut start_row__ = None;
                let mut end_row__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::RowRoots => {
                            if row_roots__.is_some() {
                                return Err(serde::de::Error::duplicate_field("rowRoots"));
                            }
                            row_roots__ = 
                                Some(map_.next_value::<Vec<::pbjson::private::BytesDeserialize<_>>>()?
                                    .into_iter().map(|x| x.0).collect())
                            ;
                        }
                        GeneratedField::Proofs => {
                            if proofs__.is_some() {
                                return Err(serde::de::Error::duplicate_field("proofs"));
                            }
                            proofs__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Root => {
                            if root__.is_some() {
                                return Err(serde::de::Error::duplicate_field("root"));
                            }
                            root__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::StartRow => {
                            if start_row__.is_some() {
                                return Err(serde::de::Error::duplicate_field("startRow"));
                            }
                            start_row__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::EndRow => {
                            if end_row__.is_some() {
                                return Err(serde::de::Error::duplicate_field("endRow"));
                            }
                            end_row__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(RowProof {
                    row_roots: row_roots__.unwrap_or_default(),
                    proofs: proofs__.unwrap_or_default(),
                    root: root__.unwrap_or_default(),
                    start_row: start_row__.unwrap_or_default(),
                    end_row: end_row__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.core.v1.proof.RowProof", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ShareProof {
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
        let mut struct_ser = serializer.serialize_struct("celestia.core.v1.proof.ShareProof", len)?;
        if true {
            struct_ser.serialize_field("data", &self.data.iter().map(pbjson::private::base64::encode).collect::<Vec<_>>())?;
        }
        if true {
            struct_ser.serialize_field("share_proofs", &self.share_proofs)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("namespace_id", pbjson::private::base64::encode(&self.namespace_id).as_str())?;
        }
        if let Some(v) = self.row_proof.as_ref() {
            struct_ser.serialize_field("row_proof", v)?;
        }
        if true {
            struct_ser.serialize_field("namespace_version", &self.namespace_version)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ShareProof {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "data",
            "share_proofs",
            "shareProofs",
            "namespace_id",
            "namespaceId",
            "row_proof",
            "rowProof",
            "namespace_version",
            "namespaceVersion",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Data,
            ShareProofs,
            NamespaceId,
            RowProof,
            NamespaceVersion,
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
                            "data" => Ok(GeneratedField::Data),
                            "shareProofs" | "share_proofs" => Ok(GeneratedField::ShareProofs),
                            "namespaceId" | "namespace_id" => Ok(GeneratedField::NamespaceId),
                            "rowProof" | "row_proof" => Ok(GeneratedField::RowProof),
                            "namespaceVersion" | "namespace_version" => Ok(GeneratedField::NamespaceVersion),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ShareProof;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.core.v1.proof.ShareProof")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ShareProof, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut data__ = None;
                let mut share_proofs__ = None;
                let mut namespace_id__ = None;
                let mut row_proof__ = None;
                let mut namespace_version__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Data => {
                            if data__.is_some() {
                                return Err(serde::de::Error::duplicate_field("data"));
                            }
                            data__ = 
                                Some(map_.next_value::<Vec<::pbjson::private::BytesDeserialize<_>>>()?
                                    .into_iter().map(|x| x.0).collect())
                            ;
                        }
                        GeneratedField::ShareProofs => {
                            if share_proofs__.is_some() {
                                return Err(serde::de::Error::duplicate_field("shareProofs"));
                            }
                            share_proofs__ = Some(map_.next_value()?);
                        }
                        GeneratedField::NamespaceId => {
                            if namespace_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("namespaceId"));
                            }
                            namespace_id__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::RowProof => {
                            if row_proof__.is_some() {
                                return Err(serde::de::Error::duplicate_field("rowProof"));
                            }
                            row_proof__ = map_.next_value()?;
                        }
                        GeneratedField::NamespaceVersion => {
                            if namespace_version__.is_some() {
                                return Err(serde::de::Error::duplicate_field("namespaceVersion"));
                            }
                            namespace_version__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(ShareProof {
                    data: data__.unwrap_or_default(),
                    share_proofs: share_proofs__.unwrap_or_default(),
                    namespace_id: namespace_id__.unwrap_or_default(),
                    row_proof: row_proof__,
                    namespace_version: namespace_version__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.core.v1.proof.ShareProof", FIELDS, GeneratedVisitor)
    }
}
