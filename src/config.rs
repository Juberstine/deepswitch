use std::{
    collections::BTreeMap,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
};

use chrono::Utc;
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tempfile::NamedTempFile;
use toml_edit::{DocumentMut, Item, Table, value};

use crate::error::{AppError, Result, io_error};

pub const DEEPSEEK_MODEL_FLASH: &str = "deepseek-v4-flash";
pub const DEEPSEEK_MODEL_PRO: &str = "deepseek-v4-pro";
const DEEPSEEK_MODELS: [&str; 2] = [DEEPSEEK_MODEL_FLASH, DEEPSEEK_MODEL_PRO];
const DEEPSEEK_PROVIDER: &str = "deepseek";
const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com/";
const STATE_VERSION: u8 = 1;
const MODEL_CATALOG: &str = include_str!("../assets/deepseek-models.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeepSeekModel {
    Flash,
    Pro,
}

impl DeepSeekModel {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Flash => DEEPSEEK_MODEL_FLASH,
            Self::Pro => DEEPSEEK_MODEL_PRO,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Flash => "DeepSeek V4 Flash",
            Self::Pro => "DeepSeek V4 Pro",
        }
    }
}

const MANAGED_SELECTION_FIELDS: [&str; 4] = [
    "model",
    "model_provider",
    "model_reasoning_effort",
    "model_catalog_json",
];
const SAVED_SELECTION_FIELDS: [&str; 6] = [
    "model",
    "model_provider",
    "preferred_auth_method",
    "forced_login_method",
    "model_reasoning_effort",
    "model_catalog_json",
];
const LEGACY_AUTH_OVERRIDES: [(&str, &str); 2] = [
    ("preferred_auth_method", "apikey"),
    ("forced_login_method", "api"),
];

#[derive(Debug, Clone)]
pub struct CodexPaths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub models: PathBuf,
    pub switcher: PathBuf,
    pub state: PathBuf,
    pub backups: PathBuf,
}

impl CodexPaths {
    pub fn discover() -> Result<Self> {
        let home = if let Some(path) = env::var_os("CODEX_HOME") {
            let path = PathBuf::from(path);
            if path.as_os_str().is_empty() || !path.is_absolute() {
                return Err(AppError::InvalidCodexHome);
            }
            path
        } else {
            UserDirs::new()
                .ok_or(AppError::HomeDirectoryUnavailable)?
                .home_dir()
                .join(".codex")
        };
        Ok(Self::from_codex_home(home))
    }

    pub fn from_codex_home(home: PathBuf) -> Self {
        let switcher = home.join("deepseek-switcher");
        Self {
            config: home.join("config.toml"),
            models: home.join("models.json"),
            state: switcher.join("state.json"),
            backups: switcher.join("backups"),
            switcher,
            home,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedState {
    version: u8,
    captured_at: String,
    selection: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeReport {
    pub changed: bool,
    pub backup: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigStatus {
    pub provider: String,
    pub model: Option<String>,
    pub deepseek_installed: bool,
    pub original_state_saved: bool,
}

pub fn switch_to_deepseek(
    paths: &CodexPaths,
    helper: &Path,
    model: DeepSeekModel,
) -> Result<ChangeReport> {
    validate_embedded_catalog()?;
    let mut document = read_config(paths)?;
    ensure_original_state(paths, &document)?;
    restore_legacy_auth_overrides(&mut document, &read_state(paths)?, &paths.state)?;

    set_string(&mut document, "model", model.slug());
    set_string(&mut document, "model_provider", DEEPSEEK_PROVIDER);
    set_string(&mut document, "model_reasoning_effort", "high");
    set_string(
        &mut document,
        "model_catalog_json",
        &paths.models.to_string_lossy(),
    );
    install_provider(&mut document, helper)?;

    let config_contents = document.to_string();
    let models_contents = pretty_catalog()?;
    let config_changed = file_differs(&paths.config, config_contents.as_bytes())?;
    let models_changed = file_differs(&paths.models, models_contents.as_bytes())?;

    if !config_changed && !models_changed {
        return Ok(ChangeReport {
            changed: false,
            backup: None,
        });
    }

    let backup = backup_current(paths)?;
    if models_changed {
        atomic_write(&paths.models, models_contents.as_bytes())?;
    }
    if config_changed {
        atomic_write(&paths.config, config_contents.as_bytes())?;
    }

    Ok(ChangeReport {
        changed: true,
        backup,
    })
}

pub fn switch_to_codex(paths: &CodexPaths) -> Result<ChangeReport> {
    let state = read_state(paths)?;
    let mut document = read_config(paths)?;
    restore_legacy_auth_overrides(&mut document, &state, &paths.state)?;

    for field in MANAGED_SELECTION_FIELDS {
        match state.selection.get(field) {
            Some(Some(original)) => {
                restore_item(&mut document, field, original, &paths.state)?;
            }
            Some(None) => {
                document.remove(field);
            }
            None => {
                return Err(AppError::UnsupportedConfigStructure(format!(
                    "saved state does not contain `{field}`"
                )));
            }
        }
    }
    let contents = document.to_string();
    if !file_differs(&paths.config, contents.as_bytes())? {
        return Ok(ChangeReport {
            changed: false,
            backup: None,
        });
    }

    let backup = backup_current(paths)?;
    atomic_write(&paths.config, contents.as_bytes())?;
    Ok(ChangeReport {
        changed: true,
        backup,
    })
}

pub fn status(paths: &CodexPaths) -> Result<ConfigStatus> {
    let document = read_config(paths)?;
    let provider =
        get_optional_string(&document, "model_provider")?.unwrap_or_else(|| "openai".to_owned());
    let model = get_optional_string(&document, "model")?;
    let deepseek_installed = document
        .get("model_providers")
        .and_then(Item::as_table)
        .is_some_and(|providers| providers.contains_key(DEEPSEEK_PROVIDER));

    Ok(ConfigStatus {
        provider,
        model,
        deepseek_installed,
        original_state_saved: paths.state.exists(),
    })
}

fn ensure_original_state(paths: &CodexPaths, document: &DocumentMut) -> Result<()> {
    if paths.state.exists() {
        read_state(paths)?;
        return Ok(());
    }
    if get_optional_string(document, "model_provider")?.as_deref() == Some(DEEPSEEK_PROVIDER) {
        return Err(AppError::DeepSeekSelectedWithoutState);
    }

    let mut selection = BTreeMap::new();
    for field in SAVED_SELECTION_FIELDS {
        selection.insert(field.to_owned(), get_optional_item(document, field)?);
    }
    let state = SavedState {
        version: STATE_VERSION,
        captured_at: Utc::now().to_rfc3339(),
        selection,
    };
    let contents = serde_json::to_vec_pretty(&state).map_err(|source| AppError::Json {
        path: paths.state.clone(),
        source,
    })?;
    ensure_private_directory(&paths.switcher)?;
    atomic_write(&paths.state, &contents)
}

fn read_state(paths: &CodexPaths) -> Result<SavedState> {
    if !paths.state.exists() {
        return Err(AppError::OriginalStateMissing);
    }
    reject_symlink(&paths.state)?;
    let contents =
        fs::read(&paths.state).map_err(|source| io_error(paths.state.clone(), source))?;
    let state: SavedState = serde_json::from_slice(&contents).map_err(|source| AppError::Json {
        path: paths.state.clone(),
        source,
    })?;
    if state.version != STATE_VERSION {
        return Err(AppError::UnsupportedConfigStructure(format!(
            "state version {} is unsupported",
            state.version
        )));
    }
    Ok(state)
}

fn read_config(paths: &CodexPaths) -> Result<DocumentMut> {
    if !paths.config.exists() {
        return Ok(DocumentMut::new());
    }
    reject_symlink(&paths.config)?;
    let contents =
        fs::read_to_string(&paths.config).map_err(|source| io_error(&paths.config, source))?;
    DocumentMut::from_str(&contents).map_err(|source| AppError::Toml {
        path: paths.config.clone(),
        source,
    })
}

fn install_provider(document: &mut DocumentMut, helper: &Path) -> Result<()> {
    let providers = document
        .entry("model_providers")
        .or_insert_with(|| Item::Table(Table::new()));
    let providers = providers.as_table_mut().ok_or_else(|| {
        AppError::UnsupportedConfigStructure("`model_providers` must be a table".to_owned())
    })?;

    let provider = providers
        .entry(DEEPSEEK_PROVIDER)
        .or_insert_with(|| Item::Table(Table::new()));
    let provider = provider.as_table_mut().ok_or_else(|| {
        AppError::UnsupportedConfigStructure(
            "`model_providers.deepseek` must be a table".to_owned(),
        )
    })?;
    provider.insert("name", value("deepseek"));
    provider.insert("base_url", value(DEEPSEEK_BASE_URL));
    provider.insert("wire_api", value("responses"));
    for conflicting_auth in [
        "env_key",
        "env_key_instructions",
        "experimental_bearer_token",
        "requires_openai_auth",
    ] {
        provider.remove(conflicting_auth);
    }

    let mut auth = Table::new();
    auth.insert("command", value(helper.to_string_lossy().as_ref()));
    let mut args = toml_edit::Array::new();
    args.push("credential");
    args.push("print");
    auth.insert("args", value(args));
    auth.insert("timeout_ms", value(5_000));
    auth.insert("refresh_interval_ms", value(0));
    provider.insert("auth", Item::Table(auth));

    Ok(())
}

fn get_optional_string(document: &DocumentMut, key: &str) -> Result<Option<String>> {
    let Some(item) = document.get(key) else {
        return Ok(None);
    };
    item.as_value()
        .and_then(toml_edit::Value::as_str)
        .map(|value| Some(value.to_owned()))
        .ok_or_else(|| AppError::UnsupportedSetting(key.to_owned()))
}

fn get_optional_item(document: &DocumentMut, key: &str) -> Result<Option<String>> {
    let Some(item) = document.get(key) else {
        return Ok(None);
    };
    if item.as_value().and_then(toml_edit::Value::as_str).is_none() {
        return Err(AppError::UnsupportedSetting(key.to_owned()));
    }
    Ok(Some(item.to_string()))
}

fn restore_legacy_auth_overrides(
    document: &mut DocumentMut,
    state: &SavedState,
    state_path: &Path,
) -> Result<()> {
    for (field, legacy_value) in LEGACY_AUTH_OVERRIDES {
        if get_optional_string(document, field)?.as_deref() != Some(legacy_value) {
            continue;
        }

        match state.selection.get(field) {
            Some(Some(original)) => {
                restore_item(document, field, original, state_path)?;
            }
            Some(None) => {
                document.remove(field);
            }
            None => {}
        }
    }
    Ok(())
}

fn restore_item(
    document: &mut DocumentMut,
    key: &str,
    representation: &str,
    state_path: &Path,
) -> Result<()> {
    let snippet = format!("{key} = {representation}\n");
    let mut parsed = DocumentMut::from_str(&snippet).map_err(|source| AppError::Toml {
        path: state_path.to_owned(),
        source,
    })?;
    let item = parsed.remove(key).ok_or_else(|| {
        AppError::UnsupportedConfigStructure(format!("saved state item `{key}` is invalid"))
    })?;
    document[key] = item;
    Ok(())
}

fn set_string(document: &mut DocumentMut, key: &str, string: &str) {
    document[key] = value(string);
}

fn validate_embedded_catalog() -> Result<()> {
    let catalog: JsonValue = serde_json::from_str(MODEL_CATALOG)
        .map_err(|error| AppError::InvalidEmbeddedCatalog(error.to_string()))?;
    let models = catalog
        .get("models")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| AppError::InvalidEmbeddedCatalog("missing `models` array".to_owned()))?;
    for expected in DEEPSEEK_MODELS {
        let present = models
            .iter()
            .any(|model| model.get("slug").and_then(JsonValue::as_str) == Some(expected));
        if !present {
            return Err(AppError::InvalidEmbeddedCatalog(format!(
                "missing model `{expected}`"
            )));
        }
    }
    Ok(())
}

fn pretty_catalog() -> Result<String> {
    let catalog: JsonValue = serde_json::from_str(MODEL_CATALOG)
        .map_err(|error| AppError::InvalidEmbeddedCatalog(error.to_string()))?;
    let mut rendered = serde_json::to_string_pretty(&catalog)
        .map_err(|error| AppError::InvalidEmbeddedCatalog(error.to_string()))?;
    rendered.push('\n');
    Ok(rendered)
}

fn file_differs(path: &Path, desired: &[u8]) -> Result<bool> {
    match fs::read(path) {
        Ok(current) => Ok(current != desired),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(io_error(path, error)),
    }
}

fn backup_current(paths: &CodexPaths) -> Result<Option<PathBuf>> {
    let files = [
        (&paths.config, "config.toml"),
        (&paths.models, "models.json"),
    ];
    if !files.iter().any(|(path, _)| path.exists()) {
        return Ok(None);
    }

    ensure_private_directory(&paths.backups)?;
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ").to_string();
    let mut backup = paths.backups.join(&timestamp);
    let mut suffix = 1;
    while backup.exists() {
        backup = paths.backups.join(format!("{timestamp}-{suffix}"));
        suffix += 1;
    }
    ensure_private_directory(&backup)?;

    for (source, filename) in files {
        if source.exists() {
            reject_symlink(source)?;
            let destination = backup.join(filename);
            if filename == "config.toml" {
                let contents = sanitized_config_backup(source)?;
                atomic_write(&destination, contents.as_bytes())?;
            } else {
                fs::copy(source, &destination)
                    .map_err(|error| io_error(destination.clone(), error))?;
                set_private_file_permissions(&destination)?;
            }
        }
    }
    Ok(Some(backup))
}

fn sanitized_config_backup(path: &Path) -> Result<String> {
    let contents = fs::read_to_string(path).map_err(|source| io_error(path, source))?;
    let mut document = DocumentMut::from_str(&contents).map_err(|source| AppError::Toml {
        path: path.to_owned(),
        source,
    })?;
    if let Some(provider) = document
        .get_mut("model_providers")
        .and_then(Item::as_table_mut)
        .and_then(|providers| providers.get_mut(DEEPSEEK_PROVIDER))
        .and_then(Item::as_table_mut)
    {
        provider.remove("experimental_bearer_token");
    }
    Ok(document.to_string())
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    reject_symlink(path)?;
    let parent = path.parent().ok_or_else(|| {
        AppError::UnsupportedConfigStructure(format!("{} has no parent", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;

    let mut temporary = NamedTempFile::new_in(parent).map_err(|source| io_error(parent, source))?;
    set_private_file_permissions(temporary.path())?;
    temporary
        .write_all(contents)
        .map_err(|source| io_error(temporary.path(), source))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| io_error(temporary.path(), source))?;
    temporary
        .persist(path)
        .map_err(|error| io_error(path, error.error))?;
    set_private_file_permissions(path)
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(AppError::SymbolicLink(path.to_owned()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(path, error)),
    }
}

fn ensure_private_directory(path: &Path) -> Result<()> {
    reject_symlink(path)?;
    fs::create_dir_all(path).map_err(|source| io_error(path, source))?;
    set_private_directory_permissions(path)
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|source| io_error(path, source))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|source| io_error(path, source))
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, CodexPaths) {
        let directory = tempfile::tempdir().expect("temp directory");
        let paths = CodexPaths::from_codex_home(directory.path().join(".codex"));
        fs::create_dir_all(&paths.home).expect("codex home");
        (directory, paths)
    }

    fn write_config(paths: &CodexPaths, contents: &str) {
        fs::write(&paths.config, contents).expect("fixture config");
    }

    #[test]
    fn switch_preserves_unrelated_config_and_restores_selection() {
        let (_directory, paths) = fixture();
        let original = r#"# keep this comment
model = "gpt-5.4" # preserve model note
model_provider = "custom-openai"
model_reasoning_effort = "medium"

[mcp_servers.example]
command = "example"
"#;
        write_config(&paths, original);

        let report =
            switch_to_deepseek(&paths, Path::new("/opt/bin/switcher"), DeepSeekModel::Flash)
                .expect("switch deepseek");
        assert!(report.changed);
        let deepseek = fs::read_to_string(&paths.config).expect("deepseek config");
        assert!(deepseek.contains("# keep this comment"));
        assert!(deepseek.contains("[mcp_servers.example]"));
        assert!(deepseek.contains("command = \"/opt/bin/switcher\""));
        assert!(!deepseek.contains("sk-secret"));

        switch_to_codex(&paths).expect("switch codex");
        let restored = fs::read_to_string(&paths.config).expect("restored config");
        let document = DocumentMut::from_str(&restored).expect("valid config");
        assert_eq!(
            get_optional_string(&document, "model").expect("model"),
            Some("gpt-5.4".to_owned())
        );
        assert_eq!(
            get_optional_string(&document, "model_provider").expect("provider"),
            Some("custom-openai".to_owned())
        );
        assert_eq!(
            get_optional_string(&document, "model_reasoning_effort").expect("effort"),
            Some("medium".to_owned())
        );
        assert_eq!(
            get_optional_string(&document, "preferred_auth_method").expect("auth"),
            None
        );
        assert!(restored.contains("[mcp_servers.example]"));
        assert!(restored.contains("# preserve model note"));
    }

    #[test]
    fn repeated_switch_is_idempotent() {
        let (_directory, paths) = fixture();
        write_config(&paths, "model = \"gpt-5.4\"\n");

        assert!(
            switch_to_deepseek(&paths, Path::new("/bin/switcher"), DeepSeekModel::Flash)
                .expect("first switch")
                .changed
        );
        assert!(
            !switch_to_deepseek(&paths, Path::new("/bin/switcher"), DeepSeekModel::Flash)
                .expect("second switch")
                .changed
        );
        assert!(
            switch_to_deepseek(&paths, Path::new("/bin/switcher"), DeepSeekModel::Pro)
                .expect("switch to pro")
                .changed
        );
        let document = read_config(&paths).expect("pro config");
        assert_eq!(
            get_optional_string(&document, "model").expect("model"),
            Some(DEEPSEEK_MODEL_PRO.to_owned())
        );
        assert!(paths.models.exists());
        let catalog = fs::read_to_string(&paths.models).expect("models");
        assert!(catalog.contains(DEEPSEEK_MODEL_FLASH));
        assert!(catalog.contains(DEEPSEEK_MODEL_PRO));
        assert!(switch_to_codex(&paths).expect("first restore").changed);
        assert!(!switch_to_codex(&paths).expect("second restore").changed);
    }

    #[test]
    fn switching_does_not_change_chatgpt_login_configuration() {
        let (_directory, paths) = fixture();
        write_config(
            &paths,
            r#"model = "gpt-5.4"
forced_login_method = "chatgpt"
cli_auth_credentials_store = "file"
"#,
        );

        switch_to_deepseek(&paths, Path::new("/bin/switcher"), DeepSeekModel::Flash)
            .expect("switch deepseek");
        let deepseek = read_config(&paths).expect("deepseek config");
        assert_eq!(
            get_optional_string(&deepseek, "forced_login_method").expect("login method"),
            Some("chatgpt".to_owned())
        );
        assert_eq!(
            get_optional_string(&deepseek, "cli_auth_credentials_store").expect("credential store"),
            Some("file".to_owned())
        );
        assert_eq!(
            get_optional_string(&deepseek, "preferred_auth_method").expect("preferred auth"),
            None
        );

        switch_to_codex(&paths).expect("switch codex");
        let codex = read_config(&paths).expect("codex config");
        assert_eq!(
            get_optional_string(&codex, "forced_login_method").expect("login method"),
            Some("chatgpt".to_owned())
        );
        assert_eq!(
            get_optional_string(&codex, "cli_auth_credentials_store").expect("credential store"),
            Some("file".to_owned())
        );
    }

    #[test]
    fn removes_auth_overrides_written_by_legacy_switcher() {
        let (_directory, paths) = fixture();
        write_config(&paths, "model = \"gpt-5.4\"\n");
        ensure_original_state(&paths, &read_config(&paths).expect("original config"))
            .expect("capture original state");
        write_config(
            &paths,
            r#"model = "deepseek-v4-flash"
model_provider = "deepseek"
preferred_auth_method = "apikey"
forced_login_method = "api"
cli_auth_credentials_store = "keyring"
"#,
        );

        switch_to_deepseek(&paths, Path::new("/bin/switcher"), DeepSeekModel::Flash)
            .expect("migrate deepseek");
        let migrated = read_config(&paths).expect("migrated config");
        assert_eq!(
            get_optional_string(&migrated, "preferred_auth_method").expect("preferred auth"),
            None
        );
        assert_eq!(
            get_optional_string(&migrated, "forced_login_method").expect("login method"),
            None
        );
        assert_eq!(
            get_optional_string(&migrated, "cli_auth_credentials_store").expect("credential store"),
            Some("keyring".to_owned())
        );

        write_config(
            &paths,
            r#"model = "deepseek-v4-flash"
model_provider = "deepseek"
preferred_auth_method = "apikey"
forced_login_method = "api"
"#,
        );
        switch_to_codex(&paths).expect("migrate codex");
        let restored = read_config(&paths).expect("restored config");
        assert_eq!(
            get_optional_string(&restored, "preferred_auth_method").expect("preferred auth"),
            None
        );
        assert_eq!(
            get_optional_string(&restored, "forced_login_method").expect("login method"),
            None
        );
        assert_eq!(
            get_optional_string(&restored, "model").expect("model"),
            Some("gpt-5.4".to_owned())
        );
    }

    #[test]
    fn malformed_config_is_not_modified() {
        let (_directory, paths) = fixture();
        let malformed = "model = [";
        write_config(&paths, malformed);

        assert!(
            switch_to_deepseek(&paths, Path::new("/bin/switcher"), DeepSeekModel::Flash).is_err()
        );
        assert_eq!(
            fs::read_to_string(&paths.config).expect("config"),
            malformed
        );
        assert!(!paths.state.exists());
        assert!(!paths.models.exists());
    }

    #[test]
    fn backs_up_existing_files_before_switching() {
        let (_directory, paths) = fixture();
        write_config(&paths, "model = \"gpt-5.4\"\n");
        fs::write(&paths.models, "{\"old\":true}\n").expect("old models");

        let report = switch_to_deepseek(&paths, Path::new("/bin/switcher"), DeepSeekModel::Flash)
            .expect("switch deepseek");
        let backup = report.backup.expect("backup path");
        assert_eq!(
            fs::read_to_string(backup.join("config.toml")).expect("config backup"),
            "model = \"gpt-5.4\"\n"
        );
        assert_eq!(
            fs::read_to_string(backup.join("models.json")).expect("models backup"),
            "{\"old\":true}\n"
        );
    }

    #[test]
    fn removes_plaintext_deepseek_tokens_from_config_and_backups() {
        let (_directory, paths) = fixture();
        let secret = "sk-must-not-be-written";
        write_config(
            &paths,
            &format!(
                r#"model_provider = "openai"

[model_providers.deepseek]
name = "deepseek"
experimental_bearer_token = "{secret}"
request_max_retries = 8
"#
            ),
        );

        let report = switch_to_deepseek(&paths, Path::new("/bin/switcher"), DeepSeekModel::Flash)
            .expect("switch deepseek");
        let backup = report.backup.expect("backup path");
        for path in [
            paths.config.clone(),
            paths.state.clone(),
            paths.models.clone(),
            backup.join("config.toml"),
        ] {
            let contents = fs::read_to_string(&path).expect("generated file");
            assert!(
                !contents.contains(secret),
                "secret leaked into {}",
                path.display()
            );
        }
        let config = fs::read_to_string(&paths.config).expect("configured file");
        assert!(config.contains("request_max_retries = 8"));
    }

    #[cfg(unix)]
    #[test]
    fn generated_files_have_restrictive_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (_directory, paths) = fixture();
        write_config(&paths, "model = \"gpt-5.4\"\n");
        switch_to_deepseek(&paths, Path::new("/bin/switcher"), DeepSeekModel::Flash)
            .expect("switch deepseek");

        for path in [&paths.config, &paths.models, &paths.state] {
            let mode = fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "unexpected permissions for {}", path.display());
        }
    }

    #[test]
    fn refuses_to_capture_deepseek_as_original_provider() {
        let (_directory, paths) = fixture();
        write_config(&paths, "model_provider = \"deepseek\"\n");

        assert!(matches!(
            switch_to_deepseek(&paths, Path::new("/bin/switcher"), DeepSeekModel::Flash),
            Err(AppError::DeepSeekSelectedWithoutState)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_replace_symlinked_config() {
        use std::os::unix::fs::symlink;

        let (directory, paths) = fixture();
        let target = directory.path().join("actual-config.toml");
        fs::write(&target, "model = \"gpt-5.4\"\n").expect("target config");
        symlink(&target, &paths.config).expect("config symlink");

        assert!(matches!(
            switch_to_deepseek(&paths, Path::new("/bin/switcher"), DeepSeekModel::Flash),
            Err(AppError::SymbolicLink(path)) if path == paths.config
        ));
        assert_eq!(
            fs::read_to_string(&target).expect("target remains"),
            "model = \"gpt-5.4\"\n"
        );
    }
}
