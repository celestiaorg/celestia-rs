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
        let mut struct_ser = serializer.serialize_struct("celestia.mint.v1.GenesisState", len)?;
        if true {
            struct_ser.serialize_field("bond_denom", &self.bond_denom)?;
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
            "bond_denom",
            "bondDenom",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            BondDenom,
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
                            "bondDenom" | "bond_denom" => Ok(GeneratedField::BondDenom),
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
                formatter.write_str("struct celestia.mint.v1.GenesisState")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<GenesisState, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut bond_denom__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::BondDenom => {
                            if bond_denom__.is_some() {
                                return Err(serde::de::Error::duplicate_field("bondDenom"));
                            }
                            bond_denom__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(GenesisState {
                    bond_denom: bond_denom__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.mint.v1.GenesisState", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for GenesisTime {
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
        let mut struct_ser = serializer.serialize_struct("celestia.mint.v1.GenesisTime", len)?;
        if let Some(v) = self.genesis_time.as_ref() {
            struct_ser.serialize_field("genesis_time", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for GenesisTime {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "genesis_time",
            "genesisTime",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            GenesisTime,
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
                            "genesisTime" | "genesis_time" => Ok(GeneratedField::GenesisTime),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = GenesisTime;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.mint.v1.GenesisTime")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<GenesisTime, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut genesis_time__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::GenesisTime => {
                            if genesis_time__.is_some() {
                                return Err(serde::de::Error::duplicate_field("genesisTime"));
                            }
                            genesis_time__ = map_.next_value()?;
                        }
                    }
                }
                Ok(GenesisTime {
                    genesis_time: genesis_time__,
                })
            }
        }
        deserializer.deserialize_struct("celestia.mint.v1.GenesisTime", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Minter {
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
        let mut struct_ser = serializer.serialize_struct("celestia.mint.v1.Minter", len)?;
        if true {
            struct_ser.serialize_field("inflation_rate", &self.inflation_rate)?;
        }
        if true {
            struct_ser.serialize_field("annual_provisions", &self.annual_provisions)?;
        }
        if let Some(v) = self.previous_block_time.as_ref() {
            struct_ser.serialize_field("previous_block_time", v)?;
        }
        if true {
            struct_ser.serialize_field("bond_denom", &self.bond_denom)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Minter {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "inflation_rate",
            "inflationRate",
            "annual_provisions",
            "annualProvisions",
            "previous_block_time",
            "previousBlockTime",
            "bond_denom",
            "bondDenom",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            InflationRate,
            AnnualProvisions,
            PreviousBlockTime,
            BondDenom,
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
                            "inflationRate" | "inflation_rate" => Ok(GeneratedField::InflationRate),
                            "annualProvisions" | "annual_provisions" => Ok(GeneratedField::AnnualProvisions),
                            "previousBlockTime" | "previous_block_time" => Ok(GeneratedField::PreviousBlockTime),
                            "bondDenom" | "bond_denom" => Ok(GeneratedField::BondDenom),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Minter;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.mint.v1.Minter")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Minter, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut inflation_rate__ = None;
                let mut annual_provisions__ = None;
                let mut previous_block_time__ = None;
                let mut bond_denom__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::InflationRate => {
                            if inflation_rate__.is_some() {
                                return Err(serde::de::Error::duplicate_field("inflationRate"));
                            }
                            inflation_rate__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AnnualProvisions => {
                            if annual_provisions__.is_some() {
                                return Err(serde::de::Error::duplicate_field("annualProvisions"));
                            }
                            annual_provisions__ = Some(map_.next_value()?);
                        }
                        GeneratedField::PreviousBlockTime => {
                            if previous_block_time__.is_some() {
                                return Err(serde::de::Error::duplicate_field("previousBlockTime"));
                            }
                            previous_block_time__ = map_.next_value()?;
                        }
                        GeneratedField::BondDenom => {
                            if bond_denom__.is_some() {
                                return Err(serde::de::Error::duplicate_field("bondDenom"));
                            }
                            bond_denom__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(Minter {
                    inflation_rate: inflation_rate__.unwrap_or_default(),
                    annual_provisions: annual_provisions__.unwrap_or_default(),
                    previous_block_time: previous_block_time__,
                    bond_denom: bond_denom__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.mint.v1.Minter", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryAnnualProvisionsRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("celestia.mint.v1.QueryAnnualProvisionsRequest", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryAnnualProvisionsRequest {
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
            type Value = QueryAnnualProvisionsRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.mint.v1.QueryAnnualProvisionsRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryAnnualProvisionsRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(QueryAnnualProvisionsRequest {
                })
            }
        }
        deserializer.deserialize_struct("celestia.mint.v1.QueryAnnualProvisionsRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryAnnualProvisionsResponse {
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
        let mut struct_ser = serializer.serialize_struct("celestia.mint.v1.QueryAnnualProvisionsResponse", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("annual_provisions", pbjson::private::base64::encode(&self.annual_provisions).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryAnnualProvisionsResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "annual_provisions",
            "annualProvisions",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            AnnualProvisions,
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
                            "annualProvisions" | "annual_provisions" => Ok(GeneratedField::AnnualProvisions),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryAnnualProvisionsResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.mint.v1.QueryAnnualProvisionsResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryAnnualProvisionsResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut annual_provisions__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::AnnualProvisions => {
                            if annual_provisions__.is_some() {
                                return Err(serde::de::Error::duplicate_field("annualProvisions"));
                            }
                            annual_provisions__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(QueryAnnualProvisionsResponse {
                    annual_provisions: annual_provisions__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.mint.v1.QueryAnnualProvisionsResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryGenesisTimeRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("celestia.mint.v1.QueryGenesisTimeRequest", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryGenesisTimeRequest {
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
            type Value = QueryGenesisTimeRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.mint.v1.QueryGenesisTimeRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryGenesisTimeRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(QueryGenesisTimeRequest {
                })
            }
        }
        deserializer.deserialize_struct("celestia.mint.v1.QueryGenesisTimeRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryGenesisTimeResponse {
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
        let mut struct_ser = serializer.serialize_struct("celestia.mint.v1.QueryGenesisTimeResponse", len)?;
        if let Some(v) = self.genesis_time.as_ref() {
            struct_ser.serialize_field("genesis_time", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryGenesisTimeResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "genesis_time",
            "genesisTime",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            GenesisTime,
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
                            "genesisTime" | "genesis_time" => Ok(GeneratedField::GenesisTime),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryGenesisTimeResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.mint.v1.QueryGenesisTimeResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryGenesisTimeResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut genesis_time__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::GenesisTime => {
                            if genesis_time__.is_some() {
                                return Err(serde::de::Error::duplicate_field("genesisTime"));
                            }
                            genesis_time__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryGenesisTimeResponse {
                    genesis_time: genesis_time__,
                })
            }
        }
        deserializer.deserialize_struct("celestia.mint.v1.QueryGenesisTimeResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryInflationRateRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("celestia.mint.v1.QueryInflationRateRequest", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryInflationRateRequest {
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
            type Value = QueryInflationRateRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.mint.v1.QueryInflationRateRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryInflationRateRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(QueryInflationRateRequest {
                })
            }
        }
        deserializer.deserialize_struct("celestia.mint.v1.QueryInflationRateRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryInflationRateResponse {
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
        let mut struct_ser = serializer.serialize_struct("celestia.mint.v1.QueryInflationRateResponse", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("inflation_rate", pbjson::private::base64::encode(&self.inflation_rate).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryInflationRateResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "inflation_rate",
            "inflationRate",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            InflationRate,
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
                            "inflationRate" | "inflation_rate" => Ok(GeneratedField::InflationRate),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryInflationRateResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.mint.v1.QueryInflationRateResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryInflationRateResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut inflation_rate__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::InflationRate => {
                            if inflation_rate__.is_some() {
                                return Err(serde::de::Error::duplicate_field("inflationRate"));
                            }
                            inflation_rate__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(QueryInflationRateResponse {
                    inflation_rate: inflation_rate__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.mint.v1.QueryInflationRateResponse", FIELDS, GeneratedVisitor)
    }
}
