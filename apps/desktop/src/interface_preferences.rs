//! Device-local appearance preferences are available before vault unlock.
//! Call only on a background worker; this file contains no personal vault data.
use std::{
    fs,
    io::{self, Read, Write},
    path::Path,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Preferences {
    pub reduced: bool,
    /// Off until chosen: icon requests reveal saved login sites to each website.
    pub website_icons: bool,
}

pub(super) fn load(path: &Path) -> io::Result<Preferences> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Preferences::default()),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid interface preference",
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid interface preference"))?;
    let reduced = value["reduce_motion"].as_bool().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "Invalid interface preference")
    })?;
    // Icon requests reveal saved login sites. Version 1 stored the former default
    // of on without a user choice, so only a version 2 value counts as consent.
    let website_icons = match value.get("website_icons") {
        None => false,
        Some(icons) => {
            icons.as_bool().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Invalid interface preference")
            })? && value["version"].as_u64() == Some(2)
        }
    };
    Ok(Preferences {
        reduced,
        website_icons,
    })
}

pub(super) fn save(path: &Path, preferences: Preferences) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "No preference directory"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".interface-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(
            serde_json::json!({"version":2,"reduce_motion":preferences.reduced,"website_icons":preferences.website_icons})
                .to_string()
                .as_bytes(),
        )?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preference_persists_atomically_outside_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui/interface.json");
        assert_eq!(load(&path).unwrap(), Preferences::default());
        save(
            &path,
            Preferences {
                reduced: true,
                website_icons: false,
            },
        )
        .unwrap();
        assert_eq!(
            load(&path).unwrap(),
            Preferences {
                reduced: true,
                website_icons: false
            }
        );
        save(&path, Preferences::default()).unwrap();
        assert_eq!(load(&path).unwrap(), Preferences::default());
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }
    #[test]
    fn website_icons_stay_off_until_explicitly_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("interface.json");
        assert!(!Preferences::default().website_icons);
        // Version 1 saved the old default of on; it never recorded a choice.
        for old in [
            r#"{"version":1,"reduce_motion":true}"#,
            r#"{"version":1,"reduce_motion":true,"website_icons":true}"#,
        ] {
            fs::write(&path, old).unwrap();
            assert_eq!(
                load(&path).unwrap(),
                Preferences {
                    reduced: true,
                    website_icons: false
                }
            );
        }
        let chosen = Preferences {
            reduced: false,
            website_icons: true,
        };
        save(&path, chosen).unwrap();
        assert_eq!(load(&path).unwrap(), chosen);
    }
    #[test]
    fn malformed_or_unbounded_preferences_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("interface.json");
        for data in [
            "not json".to_owned(),
            r#"{"reduce_motion":"yes"}"#.to_owned(),
            r#"{"reduce_motion":false,"website_icons":"yes"}"#.to_owned(),
            " ".repeat(4097),
        ] {
            fs::write(&path, data).unwrap();
            assert!(load(&path).is_err());
        }
    }
}
