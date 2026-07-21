//! Stockage persistant par application.
//!
//! Les données sont écrites dans le répertoire standard de l'OS :
//!   macOS   → ~/Library/Application Support/{app_id}/
//!   Windows → %APPDATA%\{app_id}\
//!   Linux   → ~/.local/share/{app_id}/
//!
//! Deux niveaux d'accès depuis Lua :
//!   - KV   : clés/valeurs JSON dans `kv.json`, pour la config et l'état simple.
//!   - File : fichiers arbitraires dans le même répertoire, sandboxés (pas de `..`).

use serde_json::{Map, Value};
use std::path::{Component, Path, PathBuf};

/// Répertoire de données d'une application, résolu une seule fois au démarrage.
#[derive(Clone)]
pub struct AppStorage {
    pub(crate) dir: PathBuf,
}

impl AppStorage {
    /// Construit l'instance et crée le répertoire s'il n'existe pas encore.
    pub fn new(app_id: &str) -> Self {
        let dir = data_dir(app_id);
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    // ─── KV ──────────────────────────────────────────────────────────────────

    pub fn kv_get(&self, key: &str) -> Option<Value> {
        let map = self.load_kv();
        map.get(key).cloned()
    }

    pub fn kv_set(&self, key: &str, value: Value) {
        let mut map = self.load_kv();
        map.insert(key.to_owned(), value);
        self.save_kv(&map);
    }

    pub fn kv_delete(&self, key: &str) {
        let mut map = self.load_kv();
        map.remove(key);
        self.save_kv(&map);
    }

    pub fn kv_all(&self) -> Map<String, Value> {
        self.load_kv()
    }

    fn kv_path(&self) -> PathBuf {
        self.dir.join("kv.json")
    }

    fn load_kv(&self) -> Map<String, Value> {
        std::fs::read_to_string(self.kv_path())
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.into_object())
            .unwrap_or_default()
    }

    fn save_kv(&self, map: &Map<String, Value>) {
        if let Ok(json) = serde_json::to_string_pretty(map) {
            let _ = std::fs::write(self.kv_path(), json);
        }
    }

    // ─── File ─────────────────────────────────────────────────────────────────

    /// Résout `relative` dans le répertoire de données, en refusant toute
    /// tentative de sortir du sandbox (composants `..` ou chemins absolus).
    pub fn resolve(&self, relative: &str) -> Option<PathBuf> {
        let path = Path::new(relative);
        // Refuser les chemins absolus et les composants remontants.
        for component in path.components() {
            match component {
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
                _ => {}
            }
        }
        Some(self.dir.join(path))
    }

    pub fn file_read(&self, relative: &str) -> Option<String> {
        let path = self.resolve(relative)?;
        std::fs::read_to_string(path).ok()
    }

    pub fn file_write(&self, relative: &str, content: &str) -> bool {
        let path = match self.resolve(relative) {
            Some(p) => p,
            None => return false,
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, content).is_ok()
    }

    pub fn file_delete(&self, relative: &str) -> bool {
        let path = match self.resolve(relative) {
            Some(p) => p,
            None => return false,
        };
        std::fs::remove_file(path).is_ok()
    }

    pub fn dir_create(&self, relative: &str) -> bool {
        let path = match self.resolve(relative) {
            Some(p) => p,
            None => return false,
        };
        std::fs::create_dir_all(path).is_ok()
    }

    pub fn dir_list(&self, relative: &str) -> Vec<String> {
        let path = match self.resolve(relative) {
            Some(p) => p,
            None => return Vec::new(),
        };
        std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                entry.ok().and_then(|e| e.file_name().into_string().ok())
            })
            .collect()
    }
}

/// Chemin du répertoire de données selon la plateforme.
fn data_dir(app_id: &str) -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(app_id)
}

// Extension locale pour convertir Value en Map.
trait IntoObject {
    fn into_object(self) -> Option<Map<String, Value>>;
}
impl IntoObject for Value {
    fn into_object(self) -> Option<Map<String, Value>> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn temp_storage() -> AppStorage {
        let dir = env::temp_dir().join(format!("scrawler_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        AppStorage { dir }
    }

    #[test]
    fn kv_roundtrip() {
        let s = temp_storage();
        assert_eq!(s.kv_get("x"), None);
        s.kv_set("x", serde_json::json!(42));
        assert_eq!(s.kv_get("x"), Some(serde_json::json!(42)));
        s.kv_delete("x");
        assert_eq!(s.kv_get("x"), None);
    }

    #[test]
    fn file_roundtrip() {
        let s = temp_storage();
        assert!(s.file_write("notes/hello.txt", "world"));
        assert_eq!(s.file_read("notes/hello.txt").as_deref(), Some("world"));
        assert!(s.file_delete("notes/hello.txt"));
        assert_eq!(s.file_read("notes/hello.txt"), None);
    }

    #[test]
    fn sandbox_rejects_parent_traversal() {
        let s = temp_storage();
        assert_eq!(s.resolve("../etc/passwd"), None);
        assert_eq!(s.resolve("/etc/passwd"), None);
    }

    #[test]
    fn dir_list_works() {
        let s = temp_storage();
        s.file_write("a.txt", "");
        s.file_write("b.txt", "");
        let mut entries = s.dir_list("");
        entries.sort();
        assert!(entries.contains(&"a.txt".to_owned()));
        assert!(entries.contains(&"b.txt".to_owned()));
    }
}
