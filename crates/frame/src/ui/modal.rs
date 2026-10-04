use std::collections::BTreeSet;

use bevy::prelude::*;

#[derive(Resource, Debug, Default)]
pub struct ModalInput {
    claims: BTreeSet<&'static str>,
}

impl ModalInput {
    pub fn claim(&mut self, owner: &'static str) {
        self.claims.insert(owner);
    }

    pub fn release(&mut self, owner: &'static str) {
        self.claims.remove(owner);
    }

    pub fn captured(&self) -> bool {
        !self.claims.is_empty()
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModalInputSet;
