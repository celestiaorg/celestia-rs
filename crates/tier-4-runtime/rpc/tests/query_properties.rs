//! Event query parsing and matching on valid subscription and search tags.

use std::collections::BTreeMap;

use proptest::prelude::*;
use rpc::query::Query;

proptest! {
    #[test]
    fn event_queries_preserve_unicode_tags_and_match_the_named_attribute(
        event_type in "[éΩ資१]{1,8}",
        attribute in "[a-zA-Z_０-９०-९-]{1,12}",
        height in 1_u32..=u32::MAX,
    ) {
        let tag = format!("{event_type}.{attribute}");
        let query = Query::new(&format!("{tag}={height}")).unwrap();
        prop_assert_eq!(query.to_go_string(), format!("{tag} = {height}"));

        let value = vec![height.to_string()];
        let other_tag = format!("{tag}_");
        prop_assert!(query.matches(&BTreeMap::from([(tag, value.clone())])));
        prop_assert!(!query.matches(&BTreeMap::from([(other_tag, value)])));
    }
}
