//! Process-wide count of the dialogs held by every live [`DialogManager`].
//!
//! A server that drains (stops taking new calls and waits for the calls it
//! has to end) needs to know when no dialog state is left. Each manager
//! registers its dialog store here when it is built, and
//! [`live_dialog_count`] sums the stores' current lengths. Nothing is counted
//! per dialog, so the dialog paths pay nothing for it; the figure is exact
//! for each store at the moment it is read (stores are read one after the
//! other, not as one atomic snapshot).
//!
//! [`DialogManager`]: crate::manager::DialogManager

use std::sync::{Arc, Mutex, PoisonError, Weak};

use dashmap::DashMap;

use crate::dialog::{Dialog, DialogId};

type DialogStore = DashMap<DialogId, Dialog>;

static STORES: Mutex<Vec<Weak<DialogStore>>> = Mutex::new(Vec::new());

/// Register a manager's dialog store. Stores whose last owner is gone are
/// forgotten on the next registration or count.
pub(crate) fn register(store: &Arc<DialogStore>) {
    let mut stores = STORES.lock().unwrap_or_else(PoisonError::into_inner);
    stores.retain(|store| store.strong_count() > 0);
    stores.push(Arc::downgrade(store));
}

/// Dialogs (early, confirmed, or terminated but not yet removed) held right
/// now by every [`DialogManager`](crate::manager::DialogManager) in this
/// process.
///
/// It reaches zero once every call has ended and its dialog has been
/// cleaned up, which makes it a leak check for a drained server. Each call
/// briefly locks every store's shards, so read it periodically (for example
/// once a second), not per call.
pub fn live_dialog_count() -> usize {
    let mut stores = STORES.lock().unwrap_or_else(PoisonError::into_inner);
    count_live(&mut stores)
}

fn count_live(stores: &mut Vec<Weak<DialogStore>>) -> usize {
    stores.retain(|store| store.strong_count() > 0);
    stores
        .iter()
        .filter_map(Weak::upgrade)
        .map(|store| store.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::Uri;

    fn dialog(call_id: &str) -> (DialogId, Dialog) {
        let local: Uri = "sip:alice@127.0.0.1".parse().unwrap();
        let remote: Uri = "sip:bob@127.0.0.1".parse().unwrap();
        let dialog = Dialog::new(
            call_id.to_string(),
            local,
            remote,
            Some("local".to_string()),
            Some("remote".to_string()),
            false,
        );
        (dialog.id.clone(), dialog)
    }

    #[test]
    fn counts_every_live_store_and_forgets_dropped_ones() {
        let first = Arc::new(DialogStore::new());
        let second = Arc::new(DialogStore::new());
        let mut stores = vec![Arc::downgrade(&first), Arc::downgrade(&second)];
        assert_eq!(count_live(&mut stores), 0);

        for call_id in ["a", "b"] {
            let (id, dialog) = dialog(call_id);
            first.insert(id, dialog);
        }
        let (id, dialog) = dialog("c");
        second.insert(id.clone(), dialog);
        assert_eq!(count_live(&mut stores), 3);

        second.remove(&id);
        assert_eq!(count_live(&mut stores), 2);

        drop(first);
        assert_eq!(count_live(&mut stores), 0);
        assert_eq!(stores.len(), 1, "the dropped store is forgotten");
    }

    #[test]
    fn registered_stores_reach_the_process_wide_count() {
        let store = Arc::new(DialogStore::new());
        register(&store);
        let (id, dialog) = dialog("registered");
        store.insert(id, dialog);
        // Other tests may build managers concurrently; this store's own
        // dialog is part of the count for as long as the store is held.
        assert!(live_dialog_count() >= 1);
        drop(store);
    }
}
