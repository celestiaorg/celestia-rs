impl serde::Serialize for EstimateGasPriceAndUsageRequest {
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
        let mut struct_ser = serializer.serialize_struct("celestia.core.v1.gas_estimation.EstimateGasPriceAndUsageRequest", len)?;
        if true {
            let v = TxPriority::try_from(self.tx_priority)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.tx_priority)))?;
            struct_ser.serialize_field("tx_priority", &v)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("tx_bytes", pbjson::private::base64::encode(&self.tx_bytes).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EstimateGasPriceAndUsageRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "tx_priority",
            "txPriority",
            "tx_bytes",
            "txBytes",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TxPriority,
            TxBytes,
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
                            "txPriority" | "tx_priority" => Ok(GeneratedField::TxPriority),
                            "txBytes" | "tx_bytes" => Ok(GeneratedField::TxBytes),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EstimateGasPriceAndUsageRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.core.v1.gas_estimation.EstimateGasPriceAndUsageRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EstimateGasPriceAndUsageRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut tx_priority__ = None;
                let mut tx_bytes__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TxPriority => {
                            if tx_priority__.is_some() {
                                return Err(serde::de::Error::duplicate_field("txPriority"));
                            }
                            tx_priority__ = Some(map_.next_value::<TxPriority>()? as i32);
                        }
                        GeneratedField::TxBytes => {
                            if tx_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("txBytes"));
                            }
                            tx_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(EstimateGasPriceAndUsageRequest {
                    tx_priority: tx_priority__.unwrap_or_default(),
                    tx_bytes: tx_bytes__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.core.v1.gas_estimation.EstimateGasPriceAndUsageRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EstimateGasPriceAndUsageResponse {
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
        let mut struct_ser = serializer.serialize_struct("celestia.core.v1.gas_estimation.EstimateGasPriceAndUsageResponse", len)?;
        if true {
            struct_ser.serialize_field("estimated_gas_price", &self.estimated_gas_price)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("estimated_gas_used", ToString::to_string(&self.estimated_gas_used).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EstimateGasPriceAndUsageResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "estimated_gas_price",
            "estimatedGasPrice",
            "estimated_gas_used",
            "estimatedGasUsed",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            EstimatedGasPrice,
            EstimatedGasUsed,
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
                            "estimatedGasPrice" | "estimated_gas_price" => Ok(GeneratedField::EstimatedGasPrice),
                            "estimatedGasUsed" | "estimated_gas_used" => Ok(GeneratedField::EstimatedGasUsed),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EstimateGasPriceAndUsageResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.core.v1.gas_estimation.EstimateGasPriceAndUsageResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EstimateGasPriceAndUsageResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut estimated_gas_price__ = None;
                let mut estimated_gas_used__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::EstimatedGasPrice => {
                            if estimated_gas_price__.is_some() {
                                return Err(serde::de::Error::duplicate_field("estimatedGasPrice"));
                            }
                            estimated_gas_price__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::EstimatedGasUsed => {
                            if estimated_gas_used__.is_some() {
                                return Err(serde::de::Error::duplicate_field("estimatedGasUsed"));
                            }
                            estimated_gas_used__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(EstimateGasPriceAndUsageResponse {
                    estimated_gas_price: estimated_gas_price__.unwrap_or_default(),
                    estimated_gas_used: estimated_gas_used__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.core.v1.gas_estimation.EstimateGasPriceAndUsageResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EstimateGasPriceRequest {
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
        let mut struct_ser = serializer.serialize_struct("celestia.core.v1.gas_estimation.EstimateGasPriceRequest", len)?;
        if true {
            let v = TxPriority::try_from(self.tx_priority)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.tx_priority)))?;
            struct_ser.serialize_field("tx_priority", &v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EstimateGasPriceRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "tx_priority",
            "txPriority",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TxPriority,
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
                            "txPriority" | "tx_priority" => Ok(GeneratedField::TxPriority),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EstimateGasPriceRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.core.v1.gas_estimation.EstimateGasPriceRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EstimateGasPriceRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut tx_priority__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TxPriority => {
                            if tx_priority__.is_some() {
                                return Err(serde::de::Error::duplicate_field("txPriority"));
                            }
                            tx_priority__ = Some(map_.next_value::<TxPriority>()? as i32);
                        }
                    }
                }
                Ok(EstimateGasPriceRequest {
                    tx_priority: tx_priority__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.core.v1.gas_estimation.EstimateGasPriceRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EstimateGasPriceResponse {
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
        let mut struct_ser = serializer.serialize_struct("celestia.core.v1.gas_estimation.EstimateGasPriceResponse", len)?;
        if true {
            struct_ser.serialize_field("estimated_gas_price", &self.estimated_gas_price)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EstimateGasPriceResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "estimated_gas_price",
            "estimatedGasPrice",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            EstimatedGasPrice,
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
                            "estimatedGasPrice" | "estimated_gas_price" => Ok(GeneratedField::EstimatedGasPrice),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EstimateGasPriceResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.core.v1.gas_estimation.EstimateGasPriceResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EstimateGasPriceResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut estimated_gas_price__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::EstimatedGasPrice => {
                            if estimated_gas_price__.is_some() {
                                return Err(serde::de::Error::duplicate_field("estimatedGasPrice"));
                            }
                            estimated_gas_price__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(EstimateGasPriceResponse {
                    estimated_gas_price: estimated_gas_price__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.core.v1.gas_estimation.EstimateGasPriceResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for TxPriority {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let variant = match self {
            Self::Unspecified => "TX_PRIORITY_UNSPECIFIED",
            Self::Low => "TX_PRIORITY_LOW",
            Self::Medium => "TX_PRIORITY_MEDIUM",
            Self::High => "TX_PRIORITY_HIGH",
        };
        serializer.serialize_str(variant)
    }
}
impl<'de> serde::Deserialize<'de> for TxPriority {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "TX_PRIORITY_UNSPECIFIED",
            "TX_PRIORITY_LOW",
            "TX_PRIORITY_MEDIUM",
            "TX_PRIORITY_HIGH",
        ];

        struct GeneratedVisitor;

        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = TxPriority;

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
                    "TX_PRIORITY_UNSPECIFIED" => Ok(TxPriority::Unspecified),
                    "TX_PRIORITY_LOW" => Ok(TxPriority::Low),
                    "TX_PRIORITY_MEDIUM" => Ok(TxPriority::Medium),
                    "TX_PRIORITY_HIGH" => Ok(TxPriority::High),
                    _ => Err(serde::de::Error::unknown_variant(value, FIELDS)),
                }
            }
        }
        deserializer.deserialize_any(GeneratedVisitor)
    }
}
