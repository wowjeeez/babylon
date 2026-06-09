#![allow(clippy::unwrap_used)]

use babylon_core::hub::Hub;
use babylon_core::types::{AgentKind, Handle};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]
    #[test]
    fn ack_then_catchup_never_loses_or_dups(ops in proptest::collection::vec(0u8..3, 1..40)) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let hub = Hub::new_in_memory().await.unwrap();
            let code = Handle::parse("code").unwrap();
            let bob = Handle::parse("bob").unwrap();
            hub.mint_token(&code, AgentKind::Agent).await.unwrap();
            hub.mint_token(&bob, AgentKind::Agent).await.unwrap();
            for ch in ["a", "b"] {
                hub.create_channel(&code, ch, "t").await.unwrap();
                hub.join_channel(&bob, ch).await.unwrap();
            }
            let mut posted = std::collections::BTreeSet::new();
            let mut seen = std::collections::BTreeSet::new();
            for op in ops {
                match op {
                    0 => {
                        let id = hub.post(&code, "a", "note", "m", None, &[], None).await.unwrap();
                        posted.insert(id);
                    }
                    1 => {
                        let id = hub.post(&code, "b", "note", "m", None, &[], None).await.unwrap();
                        posted.insert(id);
                    }
                    _ => {
                        let cu = hub.catch_up(&bob, None, false, 5).await.unwrap();
                        for m in &cu.messages {
                            seen.insert(m.id);
                        }
                        for (ch, cur) in &cu.next_cursors {
                            hub.ack(&bob, ch, *cur).await.unwrap();
                        }
                    }
                }
            }
            loop {
                let cu = hub.catch_up(&bob, None, false, 100).await.unwrap();
                if cu.messages.is_empty() {
                    break;
                }
                for m in &cu.messages {
                    seen.insert(m.id);
                }
                for (ch, cur) in &cu.next_cursors {
                    hub.ack(&bob, ch, *cur).await.unwrap();
                }
            }
            prop_assert_eq!(seen, posted);
            Ok(())
        }).unwrap();
    }
}
