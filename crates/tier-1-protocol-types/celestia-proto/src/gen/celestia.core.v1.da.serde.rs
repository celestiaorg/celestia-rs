impl serde::Serialize for DataAvailabilityHeader {
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
        let mut struct_ser = serializer.serialize_struct("celestia.core.v1.da.DataAvailabilityHeader", len)?;
        if true {
            struct_ser.serialize_field("row_roots", &self.row_roots.iter().map(pbjson::private::base64::encode).collect::<Vec<_>>())?;
        }
        if true {
            struct_ser.serialize_field("column_roots", &self.column_roots.iter().map(pbjson::private::base64::encode).collect::<Vec<_>>())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for DataAvailabilityHeader {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "row_roots",
            "rowRoots",
            "column_roots",
            "columnRoots",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            RowRoots,
            ColumnRoots,
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
                            "columnRoots" | "column_roots" => Ok(GeneratedField::ColumnRoots),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = DataAvailabilityHeader;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.core.v1.da.DataAvailabilityHeader")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<DataAvailabilityHeader, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut row_roots__ = None;
                let mut column_roots__ = None;
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
                        GeneratedField::ColumnRoots => {
                            if column_roots__.is_some() {
                                return Err(serde::de::Error::duplicate_field("columnRoots"));
                            }
                            column_roots__ = 
                                Some(map_.next_value::<Vec<::pbjson::private::BytesDeserialize<_>>>()?
                                    .into_iter().map(|x| x.0).collect())
                            ;
                        }
                    }
                }
                Ok(DataAvailabilityHeader {
                    row_roots: row_roots__.unwrap_or_default(),
                    column_roots: column_roots__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.core.v1.da.DataAvailabilityHeader", FIELDS, GeneratedVisitor)
    }
}
