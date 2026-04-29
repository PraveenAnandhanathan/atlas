//! SCIM 2.0 user and group provisioning server (T7.3).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A SCIM user resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimUser {
    pub id: String,
    pub user_name: String,
    pub display_name: String,
    pub email: String,
    pub groups: Vec<String>,
    pub active: bool,
}

/// A SCIM group resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimGroup {
    pub id: String,
    pub display_name: String,
    pub members: Vec<String>,
}

/// In-memory SCIM server backing the `/scim/v2` endpoint.
#[derive(Default, Clone)]
pub struct ScimServer {
    users: Arc<Mutex<HashMap<String, ScimUser>>>,
    groups: Arc<Mutex<HashMap<String, ScimGroup>>>,
}

impl ScimServer {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Users ────────────────────────────────────────────────────────────

    pub fn create_user(&self, user: ScimUser) -> Result<ScimUser, String> {
        let mut store = self.users.lock().map_err(|e| e.to_string())?;
        if store.contains_key(&user.id) {
            return Err(format!("user {} already exists", user.id));
        }
        store.insert(user.id.clone(), user.clone());
        Ok(user)
    }

    pub fn get_user(&self, id: &str) -> Option<ScimUser> {
        self.users.lock().ok()?.get(id).cloned()
    }

    pub fn update_user(&self, user: ScimUser) -> Result<ScimUser, String> {
        let mut store = self.users.lock().map_err(|e| e.to_string())?;
        store.insert(user.id.clone(), user.clone());
        Ok(user)
    }

    pub fn deactivate_user(&self, id: &str) -> Result<(), String> {
        let mut store = self.users.lock().map_err(|e| e.to_string())?;
        if let Some(u) = store.get_mut(id) {
            u.active = false;
            Ok(())
        } else {
            Err(format!("user {id} not found"))
        }
    }

    pub fn list_users(&self) -> Vec<ScimUser> {
        self.users.lock().map(|s| s.values().cloned().collect()).unwrap_or_default()
    }

    // ── Groups ───────────────────────────────────────────────────────────

    pub fn create_group(&self, group: ScimGroup) -> Result<ScimGroup, String> {
        let mut store = self.groups.lock().map_err(|e| e.to_string())?;
        store.insert(group.id.clone(), group.clone());
        Ok(group)
    }

    pub fn get_group(&self, id: &str) -> Option<ScimGroup> {
        self.groups.lock().ok()?.get(id).cloned()
    }

    pub fn add_member(&self, group_id: &str, user_id: &str) -> Result<(), String> {
        let mut store = self.groups.lock().map_err(|e| e.to_string())?;
        let g = store.get_mut(group_id).ok_or_else(|| format!("group {group_id} not found"))?;
        if !g.members.contains(&user_id.to_string()) {
            g.members.push(user_id.to_string());
        }
        Ok(())
    }

    pub fn remove_member(&self, group_id: &str, user_id: &str) -> Result<(), String> {
        let mut store = self.groups.lock().map_err(|e| e.to_string())?;
        let g = store.get_mut(group_id).ok_or_else(|| format!("group {group_id} not found"))?;
        g.members.retain(|m| m != user_id);
        Ok(())
    }

    pub fn list_groups(&self) -> Vec<ScimGroup> {
        self.groups.lock().map(|s| s.values().cloned().collect()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: &str) -> ScimUser {
        ScimUser {
            id: id.into(), user_name: id.into(), display_name: "Test".into(),
            email: format!("{id}@example.com"), groups: vec![], active: true,
        }
    }

    #[test]
    fn create_and_get_user() {
        let s = ScimServer::new();
        s.create_user(user("alice")).unwrap();
        let u = s.get_user("alice").unwrap();
        assert_eq!(u.user_name, "alice");
        assert!(u.active);
    }

    #[test]
    fn deactivate_user() {
        let s = ScimServer::new();
        s.create_user(user("bob")).unwrap();
        s.deactivate_user("bob").unwrap();
        assert!(!s.get_user("bob").unwrap().active);
    }

    #[test]
    fn group_membership() {
        let s = ScimServer::new();
        s.create_user(user("carol")).unwrap();
        s.create_group(ScimGroup { id: "g1".into(), display_name: "Admins".into(), members: vec![] }).unwrap();
        s.add_member("g1", "carol").unwrap();
        assert_eq!(s.get_group("g1").unwrap().members, vec!["carol"]);
        s.remove_member("g1", "carol").unwrap();
        assert!(s.get_group("g1").unwrap().members.is_empty());
    }

    #[test]
    fn duplicate_user_create_errors() {
        let s = ScimServer::new();
        s.create_user(user("dave")).unwrap();
        assert!(s.create_user(user("dave")).is_err());
    }
}
