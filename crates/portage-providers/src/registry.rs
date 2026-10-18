//! Provider registry: persist configured providers in the catalog and load
//! them back. PR 4 knows `local`; cloud kinds arrive with PRs 6–8.

use std::path::Path;

use portage_catalog::{Catalog, LocationKind, ProviderKind};
use portage_core::ids::ProviderId;
use portage_core::Error;

use crate::local::pathcmp::{is_within, path_str, HOST_CASE_INSENSITIVE};
use crate::local::sync_roots::OverlayRoot;
use crate::traits::Provider;
use crate::local::{location_id, prepare_local, HostOverlays, LocalConfig, LocalProvider, Resolved, Role};

/// Validate a provider id: 1–64 chars of `[a-z0-9_-]`.
pub fn validate_provider_id(id: &str) -> Result<(), Error> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(Error::ProviderConfig(format!(
            "provider id {id:?} must be 1-64 chars of a-z, 0-9, '-', '_'"
        )))
    }
}

/// Default id for a local root: `local-d` for `D:\…`, else `local-<name>`.
pub fn default_local_id(root: &Path) -> String {
    let s = path_str(root);
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return format!("local-{}", (bytes[0] as char).to_ascii_lowercase());
    }
    let name: String = root
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let name = name.trim_matches('-');
    if name.is_empty() {
        "local-root".to_string()
    } else {
        format!("local-{name}")
    }
}

/// Write every detected overlay root into `overlay_roots`. Returns the count.
pub fn register_overlays(cat: &Catalog, roots: &[OverlayRoot]) -> Result<usize, Error> {
    for r in roots {
        cat.upsert_overlay_root(&path_str(&r.path), &r.provider_id, r.detector.as_str())?;
    }
    Ok(roots.len())
}

/// `portage provider add local`: K6-check the root, record volume serial,
/// removable/bus flags and roles, register overlays, and persist.
pub fn add_local(
    cat: &Catalog,
    id: &str,
    root: &Path,
    role: Option<Role>,
    host: &HostOverlays,
) -> Result<LocalProvider, Error> {
    validate_provider_id(id)?;
    if let Some(existing) = cat.provider(id)? {
        if existing.kind != ProviderKind::Local {
            return Err(Error::ProviderConfig(format!(
                "provider {id} already exists as {}",
                existing.kind
            )));
        }
    }
    // Register overlays first: even a refused add leaves doctor informed.
    register_overlays(cat, &host.roots)?;
    let config = prepare_local(root, role, host)?;

    for other in load_local_all(cat, host)? {
        if other.id().as_str() == id {
            continue;
        }
        let a = path_str(&config.root);
        let b = path_str(&other.config().root);
        let same_volume = other
            .config()
            .volume_serial
            .eq_ignore_ascii_case(&config.volume_serial);
        if same_volume
            && (is_within(&a, &b, HOST_CASE_INSENSITIVE) || is_within(&b, &a, HOST_CASE_INSENSITIVE))
        {
            return Err(Error::ProviderConfig(format!(
                "root {a} overlaps provider {} ({b}); one tree, one provider",
                other.id()
            )));
        }
    }

    save_local(cat, id, &config)?;
    Ok(LocalProvider::new(
        ProviderId::new(id),
        config,
        host.roots.clone(),
    ))
}

fn save_local(cat: &Catalog, id: &str, config: &LocalConfig) -> Result<(), Error> {
    cat.upsert_provider_config(id, ProviderKind::Local, None, &config.to_json())?;
    cat.upsert_location(
        &location_id(id, &config.volume_serial),
        id,
        LocationKind::Volume,
        Some(&config.volume_label),
        Some(&path_str(&config.root)),
    )
}

/// Persist a drive-letter change found by [`LocalProvider::resolve`].
pub fn record_resolved(
    cat: &Catalog,
    provider: &mut LocalProvider,
    resolved: &Resolved,
) -> Result<(), Error> {
    if resolved.moved {
        provider.adopt(resolved);
        save_local(cat, provider.id().as_str(), provider.config())?;
    }
    Ok(())
}

/// Load one local provider.
pub fn load_local(cat: &Catalog, id: &str, host: &HostOverlays) -> Result<LocalProvider, Error> {
    let row = cat
        .provider(id)?
        .ok_or_else(|| Error::ProviderConfig(format!("no provider {id:?}")))?;
    if row.kind != ProviderKind::Local {
        return Err(Error::ProviderConfig(format!(
            "provider {id} is {}, not local",
            row.kind
        )));
    }
    Ok(LocalProvider::new(
        ProviderId::new(row.id),
        LocalConfig::from_json(&row.config_json)?,
        host.roots.clone(),
    ))
}

/// Load every local provider, ordered by id.
pub fn load_local_all(cat: &Catalog, host: &HostOverlays) -> Result<Vec<LocalProvider>, Error> {
    cat.providers()?
        .into_iter()
        .filter(|row| row.kind == ProviderKind::Local)
        .map(|row| {
            Ok(LocalProvider::new(
                ProviderId::new(row.id),
                LocalConfig::from_json(&row.config_json)?,
                host.roots.clone(),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn provider_ids() {
        for ok in ["local-d", "ext_media", "a", "x1"] {
            validate_provider_id(ok).unwrap();
        }
        for bad in ["", "Local", "a b", "../x", &"a".repeat(65)] {
            assert!(validate_provider_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn default_ids() {
        assert_eq!(default_local_id(Path::new(r"D:\")), "local-d");
        assert_eq!(default_local_id(Path::new(r"\\?\E:\Media")), "local-e");
        assert_eq!(default_local_id(&PathBuf::from("/media/My USB")), "local-my-usb");
        assert_eq!(default_local_id(Path::new("/")), "local-root");
    }
}
