//! Thread-bound Windows Runtime initialization, balanced before thread exit.

use std::{marker::PhantomData, rc::Rc};
use windows::Win32::System::WinRT::{RO_INIT_TYPE, RoInitialize, RoUninitialize};

pub(crate) struct Apartment(PhantomData<Rc<()>>);

impl Apartment {
    pub(crate) fn new(model: RO_INIT_TYPE) -> windows::core::Result<Self> {
        // Every successful initialization (including S_FALSE) owns one
        // uninitialize call. Rc's marker prevents moving that duty to a
        // different thread. Declare this guard before apartment objects.
        unsafe { RoInitialize(model) }?;
        Ok(Self(PhantomData))
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}
