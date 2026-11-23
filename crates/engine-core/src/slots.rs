//! Session slots: state a crate above core keeps in every [`Session`], typed by that crate.
//!
//! Core's own state and the state its pipeline reads are plain `Session` fields. State that only
//! crates above core read sits in a slot instead: the owning crate registers its type in its
//! `register` ([`Install::slot`](crate::registry::Install::slot)), every session made after the
//! install holds one default value of it, and the crate reads it through its own extension trait
//! over [`Session::slot`] / [`Session::slot_mut`].

use std::any::{Any, TypeId};

use crate::{Result, Session};

/// How a session makes one slot: its type and its default value.
pub(crate) struct SlotInit {
    id: TypeId,
    make: fn() -> Box<dyn Any + Send>,
}

impl SlotInit {
    /// The slot of `T`, starting at `T::default()`.
    pub(crate) fn of<T: Default + Send + 'static>() -> Self {
        fn make<T: Default + Send + 'static>() -> Box<dyn Any + Send> {
            Box::new(T::default())
        }
        Self { id: TypeId::of::<T>(), make: make::<T> }
    }

    pub(crate) fn id(&self) -> TypeId {
        self.id
    }
}

/// A session's slot values, one per [`SlotInit`] of the process's install.
pub(crate) struct Slots(Vec<Box<dyn Any + Send>>);

impl Default for Slots {
    /// The installed slots' defaults; none before an install.
    fn default() -> Self {
        Self(crate::registry::slot_inits().iter().map(|init| (init.make)()).collect())
    }
}

impl Session {
    /// The slot of `T`. Fails with the install's error when the crate that registers `T` was not
    /// installed before this session was made.
    pub(crate) fn slot<T: Any + Send>(&self) -> Result<&T> {
        self.slots.0.iter().find_map(|value| value.downcast_ref::<T>()).ok_or_else(crate::registry::slot_error)
    }

    /// The slot of `T`, to change it; fails as [`Self::slot`] does.
    pub(crate) fn slot_mut<T: Any + Send>(&mut self) -> Result<&mut T> {
        self.slots.0.iter_mut().find_map(|value| value.downcast_mut::<T>()).ok_or_else(crate::registry::slot_error)
    }
}

// Slot values are `Send`, so a session stays `Send` (it is not `Sync`: job closures aren't).
const _: () = {
    const fn send<T: Send>() {}
    send::<Session>();
};

#[cfg(test)]
mod tests {
    use crate::EngineError;
    use crate::preset_store::PresetStore;

    #[test]
    fn a_slot_no_crate_registered_is_an_error_not_a_panic() {
        #[derive(Default)]
        struct Unregistered;
        let mut s = crate::session();
        assert!(s.slot::<Option<PresetStore>>().is_ok_and(Option::is_none), "the preset store slot is installed, empty");
        assert!(matches!(s.slot::<Unregistered>(), Err(EngineError::NotInstalled)));
        assert!(matches!(s.slot_mut::<Unregistered>(), Err(EngineError::NotInstalled)));
    }
}
