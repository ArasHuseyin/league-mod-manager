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
    fn disabling_mod_keeps_order_for_future_reenable() {
        let mod_id = Uuid::new_v4();
        let mut profile = Profile::new("Default");

        profile.enable_mod(mod_id);
        profile.disable_mod(mod_id);

        assert!(profile.enabled_mods.is_empty());
        assert_eq!(profile.mod_order, vec![mod_id]);
    }
}
