//! Persistent storage for Reef: device profiles, saved configs, change history, and local backups.
//!
//! Uses the file system (JSON) under the Reef config directory so backups and
//! history survive restarts and are independent of any device session.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

use directories::ProjectDirs;

/// A saved device connection profile.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub transport: TransportType,
    pub username: String,
}

/// Type of transport in use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportType {
    Ssh,
    Telnet,
    Serial,
}

/// A timestamped backup of a device's running-config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigBackup {
    pub id: String,
    pub device_id: String,
    pub device_name: String,
    pub created_at: String,
    pub running_config: String,
    pub source: BackupSource,
}

/// Where a backup came from.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BackupSource {
    /// Taken automatically by Reef before an apply.
    PreApply,
    /// Taken manually by the user.
    Manual,
    /// Scheduled/periodic.
    Scheduled,
}

/// A recorded change applied to a device.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangeRecord {
    pub id: String,
    pub device_id: String,
    pub device_name: String,
    pub created_at: String,
    pub diff: String,
    pub verification: String,
    pub success: bool,
    pub rolled_back: bool,
}

/// Errors for the storage layer.
#[derive(Debug, Error)]
pub enum Error {
    #[error("storage error: {0}")]
    Io(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Persist a complete JSON document through a sibling temporary file so a
/// process interruption cannot leave an existing record half-written.
fn write_json_atomic(path: &Path, json: &[u8]) -> Result<(), Error> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::Io("invalid storage file name".to_string()))?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| Error::Io(error.to_string()))?
        .as_nanos();
    let temporary_path = path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        nonce
    ));

    let write_result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .map_err(|error| Error::Io(error.to_string()))?;
        file.write_all(json)
            .map_err(|error| Error::Io(error.to_string()))?;
        file.sync_all()
            .map_err(|error| Error::Io(error.to_string()))?;
        fs::rename(&temporary_path, path).map_err(|error| Error::Io(error.to_string()))?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }

    write_result
}

fn data_dir() -> Result<PathBuf, Error> {
    ProjectDirs::from("", "", "reef")
        .map(|p| p.data_dir().to_path_buf())
        .ok_or_else(|| Error::Io("cannot determine data directory".to_string()))
}

/// Main storage handle. All operations are synchronous file I/O.
pub struct Storage {
    data_dir: PathBuf,
}

impl Storage {
    /// Open (creating if necessary) the Reef data directory.
    pub fn open() -> Result<Self, Error> {
        let dir = data_dir()?;
        fs::create_dir_all(&dir).map_err(|e| Error::Io(e.to_string()))?;
        fs::create_dir_all(dir.join("backups")).map_err(|e| Error::Io(e.to_string()))?;
        fs::create_dir_all(dir.join("profiles")).map_err(|e| Error::Io(e.to_string()))?;
        fs::create_dir_all(dir.join("history")).map_err(|e| Error::Io(e.to_string()))?;
        Ok(Storage { data_dir: dir })
    }

    fn record_path(&self, directory: &str, id: &str) -> Result<PathBuf, Error> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(Error::Io("invalid storage record id".to_string()));
        }

        Ok(self.data_dir.join(directory).join(format!("{id}.json")))
    }

    fn profile_path(&self, profile_id: &str) -> Result<PathBuf, Error> {
        self.record_path("profiles", profile_id)
    }

    /// Save a device connection profile.
    pub fn save_profile(&self, profile: &DeviceProfile) -> Result<(), Error> {
        let path = self.profile_path(&profile.id)?;
        let json = serde_json::to_string_pretty(profile)
            .map_err(|e| Error::Serialization(e.to_string()))?;
        write_json_atomic(&path, json.as_bytes())
    }

    /// Delete a saved device connection profile.
    pub fn delete_profile(&self, profile_id: &str) -> Result<(), Error> {
        let path = self.profile_path(profile_id)?;
        fs::remove_file(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::NotFound(profile_id.to_string())
            } else {
                Error::Io(error.to_string())
            }
        })
    }

    /// List all saved device profiles.
    pub fn list_profiles(&self) -> Result<Vec<DeviceProfile>, Error> {
        let profiles_dir = self.data_dir.join("profiles");
        let mut out = Vec::new();
        if profiles_dir.is_dir() {
            for entry in fs::read_dir(&profiles_dir).map_err(|e| Error::Io(e.to_string()))? {
                let entry = entry.map_err(|e| Error::Io(e.to_string()))?;
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    let data = fs::read_to_string(&path).map_err(|e| Error::Io(e.to_string()))?;
                    let profile: DeviceProfile = serde_json::from_str(&data)
                        .map_err(|e| Error::Serialization(e.to_string()))?;
                    out.push(profile);
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Save a configuration backup.
    pub fn save_backup(&self, backup: &ConfigBackup) -> Result<(), Error> {
        let path = self.record_path("backups", &backup.id)?;
        let json = serde_json::to_string_pretty(backup)
            .map_err(|e| Error::Serialization(e.to_string()))?;
        write_json_atomic(&path, json.as_bytes())
    }

    /// List all backups for a device, newest first.
    pub fn list_backups(&self, device_id: &str) -> Result<Vec<ConfigBackup>, Error> {
        let backups_dir = self.data_dir.join("backups");
        let mut out = Vec::new();
        if backups_dir.is_dir() {
            for entry in fs::read_dir(&backups_dir).map_err(|e| Error::Io(e.to_string()))? {
                let entry = entry.map_err(|e| Error::Io(e.to_string()))?;
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    let data = fs::read_to_string(&path).map_err(|e| Error::Io(e.to_string()))?;
                    let backup: ConfigBackup = serde_json::from_str(&data)
                        .map_err(|e| Error::Serialization(e.to_string()))?;
                    if backup.device_id == device_id {
                        out.push(backup);
                    }
                }
            }
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(out)
    }

    /// Get a backup by ID.
    pub fn get_backup(&self, backup_id: &str) -> Result<ConfigBackup, Error> {
        let path = self.record_path("backups", backup_id)?;
        let data = fs::read_to_string(&path).map_err(|_| Error::NotFound(backup_id.to_string()))?;
        serde_json::from_str(&data).map_err(|e| Error::Serialization(e.to_string()))
    }

    /// Record a change in the history.
    pub fn record_change(&self, record: &ChangeRecord) -> Result<(), Error> {
        let path = self.record_path("history", &record.id)?;
        let json = serde_json::to_string_pretty(record)
            .map_err(|e| Error::Serialization(e.to_string()))?;
        write_json_atomic(&path, json.as_bytes())
    }

    /// List change history for a device, newest first.
    pub fn list_history(&self, device_id: &str) -> Result<Vec<ChangeRecord>, Error> {
        let history_dir = self.data_dir.join("history");
        let mut out = Vec::new();
        if history_dir.is_dir() {
            for entry in fs::read_dir(&history_dir).map_err(|e| Error::Io(e.to_string()))? {
                let entry = entry.map_err(|e| Error::Io(e.to_string()))?;
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    let data = fs::read_to_string(&path).map_err(|e| Error::Io(e.to_string()))?;
                    let record: ChangeRecord = serde_json::from_str(&data)
                        .map_err(|e| Error::Serialization(e.to_string()))?;
                    if record.device_id == device_id {
                        out.push(record);
                    }
                }
            }
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(out)
    }

    /// Return the Reef data directory path (useful for UI display).
    pub fn data_dir_path(&self) -> &Path {
        &self.data_dir
    }
}

impl Default for Storage {
    fn default() -> Self {
        Self::open().unwrap_or_else(|_| Storage {
            data_dir: std::env::temp_dir().join("reef"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn now_str() -> String {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs().to_string())
            .unwrap_or_default()
    }

    #[test]
    fn json_records_are_replaced_atomically() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "reef-storage-atomic-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("create test directory");
        let path = directory.join("record.json");

        write_json_atomic(&path, br#"{ "value": 1 }"#).expect("write initial record");
        write_json_atomic(&path, br#"{ "value": 2 }"#).expect("replace record");

        assert_eq!(
            fs::read_to_string(&path).expect("read record"),
            r#"{ "value": 2 }"#
        );
        let leftovers = fs::read_dir(&directory)
            .expect("read test directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn storage_open_creates_dirs() {
        let s = Storage::open().expect("storage should open");
        assert!(s.data_dir.join("backups").is_dir());
        assert!(s.data_dir.join("profiles").is_dir());
        assert!(s.data_dir.join("history").is_dir());
    }

    #[test]
    fn profile_round_trip() {
        let s = Storage::open().expect("storage should open");
        let profile = DeviceProfile {
            id: "test-1".to_string(),
            name: "Test Switch".to_string(),
            host: "10.0.0.1".to_string(),
            port: 22,
            transport: TransportType::Ssh,
            username: "admin".to_string(),
        };
        s.save_profile(&profile).expect("save profile");
        let profiles = s.list_profiles().expect("list profiles");
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].host, "10.0.0.1");
    }

    #[test]
    fn profile_id_rejects_path_traversal() {
        let storage = Storage::open().expect("storage should open");
        assert!(storage.profile_path("../outside").is_err());
        assert!(storage.profile_path("").is_err());
    }

    #[test]
    fn backup_and_history_ids_reject_path_traversal() {
        let storage = Storage::open().expect("storage should open");
        assert!(storage.record_path("backups", "../outside").is_err());
        assert!(storage.record_path("history", "../../outside").is_err());
        assert!(storage.record_path("backups", "").is_err());
    }

    #[test]
    fn backup_round_trip() {
        let s = Storage::open().expect("storage should open");
        let backup = ConfigBackup {
            id: "backup-1".to_string(),
            device_id: "dev-1".to_string(),
            device_name: "Switch1".to_string(),
            created_at: now_str(),
            running_config: "hostname Switch1\n".to_string(),
            source: BackupSource::PreApply,
        };
        s.save_backup(&backup).expect("save backup");
        let backups = s.list_backups("dev-1").expect("list backups");
        assert_eq!(backups.len(), 1);
        let got = s.get_backup("backup-1").expect("get backup");
        assert_eq!(got.running_config, "hostname Switch1\n");
    }

    #[test]
    fn change_record_round_trip() {
        let s = Storage::open().expect("storage should open");
        let record = ChangeRecord {
            id: "change-1".to_string(),
            device_id: "dev-1".to_string(),
            device_name: "Switch1".to_string(),
            created_at: now_str(),
            diff: "Interface GigabitEthernet1/0/1:\n  [ADD]\n".to_string(),
            verification: "Verified".to_string(),
            success: true,
            rolled_back: false,
        };
        s.record_change(&record).expect("record change");
        let history = s.list_history("dev-1").expect("list history");
        assert_eq!(history.len(), 1);
        assert!(history[0].success);
    }

    #[test]
    fn backup_not_found() {
        let s = Storage::open().expect("storage should open");
        assert!(s.get_backup("nonexistent").is_err());
    }
}
