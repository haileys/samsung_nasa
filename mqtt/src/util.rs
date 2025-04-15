use std::cell::{Ref, RefCell, RefMut};

use derive_more::{Deref, DerefMut};
use tokio::sync::watch;

#[derive(Default)]
pub struct NotifyCell<T> {
    inner: RefCell<T>,
    notify: watch::Sender<()>,
}

impl<T> NotifyCell<T> {
    pub fn borrow(&self) -> Ref<T> {
        self.inner.borrow()
    }

    pub fn borrow_mut(&self) -> NotifyCellMut<T> {
        let ref_ = self.inner.borrow_mut();
        NotifyCellMut { ref_, cell: self }
    }

    pub fn subscribe(&self) -> watch::Receiver<()> {
        self.notify.subscribe()
    }
}

#[derive(Deref, DerefMut)]
pub struct NotifyCellMut<'a, T> {
    #[deref(forward)]
    #[deref_mut(forward)]
    ref_: RefMut<'a, T>,
    cell: &'a NotifyCell<T>,
}

impl<'a, T> Drop for NotifyCellMut<'a, T> {
    fn drop(&mut self) {
        self.cell.notify.send_replace(());
    }
}
