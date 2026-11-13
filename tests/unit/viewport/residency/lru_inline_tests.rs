use super::*;

#[test]
fn weighted_lru_evicts_oldest_unpinned_entry() {
    let mut lru = WarmLru::default();
    lru.insert(1, 10, 20, false);
    lru.insert(2, 30, 40, true);
    lru.insert(3, 50, 60, false);
    assert_eq!(lru.pop_oldest_unpinned(), Some((1, 10, 20)));
    assert!(lru.contains(&2));
    assert_eq!(lru.len(), 2);
}

#[test]
fn replacing_an_entry_refreshes_its_recency() {
    let mut lru = WarmLru::default();
    lru.insert(1, 1, 1, false);
    lru.insert(2, 1, 1, false);
    lru.insert(1, 1, 1, false);
    assert_eq!(lru.pop_oldest_unpinned().map(|entry| entry.0), Some(2));
}
