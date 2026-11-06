//! A list of named setups ("tabs") with one marked active. Generic over the
//! stored data so the list logic is unit-testable on its own.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setup<T> {
    pub name: String,
    pub data: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupList<T> {
    /// Index of the setup whose links are live in PipeWire.
    pub active: usize,
    pub items: Vec<Setup<T>>,
}

impl<T> SetupList<T> {
    pub fn new(name: &str, data: T) -> Self {
        Self { active: 0, items: vec![Setup { name: name.to_string(), data }] }
    }

    /// Make sure the list is non-empty and `active` is valid (after loading).
    pub fn fix(&mut self, fallback: impl FnOnce() -> T) {
        if self.items.is_empty() {
            self.items.push(Setup { name: "Main".to_string(), data: fallback() });
        }
        self.active = self.active.min(self.items.len() - 1);
    }

    /// `base`, or `base 2`, `base 3`... if the name is taken.
    pub fn unique_name(&self, base: &str) -> String {
        let taken = |n: &str| self.items.iter().any(|s| s.name == n);
        if !taken(base) {
            return base.to_string();
        }
        (2..).map(|k| format!("{base} {k}")).find(|n| !taken(n)).unwrap()
    }

    pub fn add(&mut self, name: &str, data: T) -> usize {
        let name = self.unique_name(name);
        self.items.push(Setup { name, data });
        self.items.len() - 1
    }

    /// Delete setup `idx`. Refuses the active or the last one. `viewing` is
    /// shifted so it keeps pointing at the same setup.
    pub fn delete(&mut self, idx: usize, viewing: &mut usize) -> bool {
        if idx >= self.items.len() || idx == self.active || self.items.len() == 1 {
            return false;
        }
        self.items.remove(idx);
        if idx < self.active {
            self.active -= 1;
        }
        if *viewing == idx {
            *viewing = self.active;
        } else if *viewing > idx {
            *viewing -= 1;
        }
        true
    }

    pub fn rename(&mut self, idx: usize, name: &str) {
        let name = name.trim();
        if name.is_empty() || self.items.iter().enumerate().any(|(i, s)| i != idx && s.name == name) {
            return;
        }
        if let Some(s) = self.items.get_mut(idx) {
            s.name = name.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> SetupList<i32> {
        let mut l = SetupList::new("Main", 0);
        l.add("Gaming", 1);
        l.add("Stream", 2);
        l
    }

    #[test]
    fn unique_names() {
        let mut l = list();
        assert_eq!(l.unique_name("New"), "New");
        let i = l.add("Gaming", 9);
        assert_eq!(l.items[i].name, "Gaming 2");
        assert_eq!(l.unique_name("Gaming"), "Gaming 3");
    }

    #[test]
    fn delete_keeps_active_and_viewing_pointing_at_same_setup() {
        let mut l = list();
        l.active = 2;
        let mut v = 2;
        assert!(!l.delete(2, &mut v)); // active is protected
        assert!(l.delete(0, &mut v));
        assert_eq!((l.active, v), (1, 1));
        assert_eq!(l.items[l.active].name, "Stream");
        // Deleting the viewed (non-active) one moves the view to the active one.
        v = 0;
        assert!(l.delete(0, &mut v));
        assert_eq!((l.active, v), (0, 0));
        assert_eq!(l.items.len(), 1);
        assert!(!l.delete(0, &mut v)); // last one stays
    }

    #[test]
    fn rename_rejects_empty_and_duplicates() {
        let mut l = list();
        l.rename(1, "  ");
        l.rename(1, "Main");
        assert_eq!(l.items[1].name, "Gaming");
        l.rename(1, " Games ");
        assert_eq!(l.items[1].name, "Games");
        l.rename(1, "Games");
        assert_eq!(l.items[1].name, "Games");
    }

    #[test]
    fn fix_repairs_loaded_data() {
        let mut l: SetupList<i32> = SetupList { active: 5, items: vec![] };
        l.fix(|| 7);
        assert_eq!((l.active, l.items.len(), l.items[0].data), (0, 1, 7));
        let mut l = list();
        l.active = 9;
        l.fix(|| 0);
        assert_eq!(l.active, 2);
    }
}
