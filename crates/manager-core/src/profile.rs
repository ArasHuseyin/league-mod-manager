use crate::manifest::ModId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type ProfileId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub enabled_mods: Vec<ModId>,
    pub mod_order: Vec<ModId>,
}

impl Profile {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            enabled_mods: Vec::new(),
            mod_order: Vec::new(),
        }
    }

    pub fn enable_mod(&mut self, mod_id: ModId) {
        if !self.enabled_mods.contains(&mod_id) {
            self.enabled_mods.push(mod_id);
        }
        if !self.mod_order.contains(&mod_id) {
            self.mod_order.push(mod_id);
        }
    }

    pub fn disable_mod(&mut self, mod_id: ModId) {
        self.enabled_mods.retain(|enabled| *enabled != mod_id);
    }

    /// Move `mod_id` one step earlier (`up`) or later in the load order. Later
    /// entries win when two enabled mods write the same asset, so this is how a
    /// user resolves an overlap in their favor. No-op if the mod is absent or
    /// already at the relevant end.
    pub fn move_mod(&mut self, mod_id: ModId, up: bool) {
        let Some(index) = self.mod_order.iter().position(|id| *id == mod_id) else {
            return;
        };
        let target = if up {
            index.checked_sub(1)
        } else if index + 1 < self.mod_order.len() {
            Some(index + 1)
        } else {
            None
        };
        if let Some(target) = target {
            self.mod_order.swap(index, target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabling_mod_adds_enabled_and_order_once() {
        let mod_id = Uuid::new_v4();
        let mut profile = Profile::new("Default");

        profile.enable_mod(mod_id);
        profile.enable_mod(mod_id);

        assert_eq!(profile.enabled_mods, vec![mod_id]);
        assert_eq!(profile.mod_order, vec![mod_id]);
    }

    #[test]
    fn move_mod_reorders_within_load_order() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let mut profile = Profile::new("Default");
        profile.enable_mod(a);
        profile.enable_mod(b);
        profile.enable_mod(c);

        profile.move_mod(c, true); // c moves ahead of b -> [a, c, b]
        assert_eq!(profile.mod_order, vec![a, c, b]);

        profile.move_mod(a, true); // already first -> unchanged
        assert_eq!(profile.mod_order, vec![a, c, b]);

        profile.move_mod(a, false); // a moves later -> [c, a, b]
        assert_eq!(profile.mod_order, vec![c, a, b]);

        profile.move_mod(Uuid::new_v4(), true); // unknown id -> no-op
        assert_eq!(profile.mod_order, vec![c, a, b]);
    }

    #[test]
    fn disabling_mod_keeps_order_for_future_reenable() {
        let mod_id = Uuid::new_v4();
        let mut profile = Profile::new("Default");

        profile.enable_mod(mod_id);
        profile.disable_mod(mod_id);

        assert!(profile.enabled_mods.is_empty());
        assert_eq!(profile.mod_order, vec![mod_id]);
    }
}
