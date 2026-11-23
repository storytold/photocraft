//! Session slots: state a crate above core keeps in every [`Session`], typed by that crate.
//!
//! Core's own state and the state its pipeline reads are plain `Session` fields. State that only
//! crates above core read sits in a slot instead: the owning crate registers its type in its
//! `register` ([`Install::slot`](crate::registry::Install::slot)), every session made after the
//! install holds one default value of it, and the crate reads it through its own extension trait
//! over [`Session::slot_or_default`] / [`Session::slot_or_default_mut`] (or [`Session::slot`] /
//! [`Session::slot_mut`], which report a missing slot).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

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

    /// The slot of `T`, or `T`'s default when this session has none. The default stands in so that a
    /// session made before the install (or after a conflicting one) reads as its fields did before
    /// they became slots; its commands still fail with the install's error.
    pub(crate) fn slot_or_default<T: Default + Any + Send + Sync>(&self) -> &T {
        match self.slot::<T>() {
            Ok(value) => value,
            Err(_) => default_of::<T>(),
        }
    }

    /// The slot of `T`, to change it. A session without one gets `T::default()` at the first write,
    /// for the reason [`Self::slot_or_default`] gives.
    pub(crate) fn slot_or_default_mut<T: Default + Any + Send>(&mut self) -> &mut T {
        let slots = &mut self.slots.0;
        let at = match slots.iter().position(|value| value.is::<T>()) {
            Some(at) => at,
            None => {
                slots.push(Box::new(T::default()));
                slots.len() - 1
            }
        };
        match slots.get_mut(at).and_then(|value| value.downcast_mut::<T>()) {
            Some(value) => value,
            // Never taken: the value at `at` is a `T`.
            None => Box::leak(Box::default()),
        }
    }
}

/// One default of `T` for the whole process, made at the first read of a slot of `T` that a
/// session lacks ([`Session::slot_or_default`]).
fn default_of<T: Default + Any + Send + Sync>() -> &'static T {
    static DEFAULTS: LazyLock<Mutex<HashMap<TypeId, &'static (dyn Any + Send + Sync)>>> = LazyLock::new(Mutex::default);
    let mut defaults = DEFAULTS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(value) = defaults.get(&TypeId::of::<T>()).and_then(|value| value.downcast_ref::<T>()) {
        return value;
    }
    let value: &'static T = Box::leak(Box::new(T::default()));
    defaults.insert(TypeId::of::<T>(), value);
    value
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

    #[test]
    fn a_session_without_the_install_reads_each_slot_at_its_default_and_keeps_a_write() {
        use crate::actions_cmds::{ActionState, ActionsExt};
        use crate::color_cmds::{ColorExt, ColorState};
        use crate::presets::{PresetState, PresetsExt};
        use crate::print_cmds::PrintExt;
        use crate::smartselect_cmds::{ObjectFinderCache, SmartSelectExt};

        let mut s = crate::session();
        // What a session made before any install holds.
        s.slots = super::Slots(Vec::new());
        assert_eq!(s.color().settings, ColorState::default().settings);
        assert_eq!(s.presets().gradient, PresetState::default().gradient);
        assert_eq!(*s.actions(), ActionState::default());
        assert_eq!(format!("{:?}", s.object_finder()), format!("{:?}", ObjectFinderCache::default()));
        assert!(s.print_spooler().is_none() && s.printer_source().is_none());
        assert!(s.slot::<ColorState>().is_err(), "a read adds no slot");

        s.color_mut().main_display = Some(3);
        s.presets_mut().pattern = Some("Mine".into());
        s.actions_mut().replaced_before_attachment = true;
        *s.object_finder_mut() = ObjectFinderCache::default();
        *s.printer_source_mut() = Some(std::sync::Arc::new(Vec::new));
        assert_eq!(s.color().main_display, Some(3));
        assert_eq!(s.presets().pattern.as_deref(), Some("Mine"));
        assert!(s.actions().replaced_before_attachment);
        assert!(s.slot::<ObjectFinderCache>().is_ok());
        assert!(s.printer_source().is_some() && s.print_spooler().is_none());
    }
}
