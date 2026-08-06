use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
    process::{Command as ProcessCommand, ExitCode},
};

use clap::{Parser, Subcommand, ValueEnum};
use deepswitch::{
    config::{self, ChangeReport, CodexPaths, DeepSeekModel},
    credentials::{
        CredentialStore, NativeCredentialStore, credential_backend_name, is_wsl_runtime,
        runtime_name,
    },
    error::{AppError, Result},
};
use secrecy::{ExposeSecret, SecretString};

#[derive(Debug, Parser)]
#[command(
    name = "deepswitch",
    version,
    about = "Safely switch Codex between OpenAI and DeepSeek"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

struct Installation {
    name: &'static str,
    paths: CodexPaths,
    companion: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Activate the saved OpenAI/Codex provider.
    Codex,
    /// Activate DeepSeek V4 Flash or Pro.
    Deepseek {
        /// DeepSeek model to activate.
        #[arg(value_enum, default_value_t = DeepSeekModelArg::Flash)]
        model: DeepSeekModelArg,
    },
    /// Store a DeepSeek key securely and activate DeepSeek.
    Setup {
        /// DeepSeek model to activate after storing the key.
        #[arg(value_enum, default_value_t = DeepSeekModelArg::Flash)]
        model: DeepSeekModelArg,
    },
    /// Activate the selected provider using the legacy syntax.
    #[command(hide = true)]
    Use {
        #[arg(value_enum)]
        provider: Provider,
    },
    /// Show the active provider and installation state.
    Status,
    /// Manage the DeepSeek API key in the OS keychain.
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
    /// Internal credential helper used by Codex.
    #[command(hide = true)]
    Credential {
        #[command(subcommand)]
        command: CredentialCommand,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Provider {
    /// Restore the OpenAI/Codex selection captured during setup.
    Codex,
    /// Use DeepSeek V4 Flash through the Responses API.
    Deepseek,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum DeepSeekModelArg {
    /// DeepSeek V4 Flash.
    Flash,
    /// DeepSeek V4 Pro.
    Pro,
}

impl From<DeepSeekModelArg> for DeepSeekModel {
    fn from(value: DeepSeekModelArg) -> Self {
        match value {
            DeepSeekModelArg::Flash => Self::Flash,
            DeepSeekModelArg::Pro => Self::Pro,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Selection {
    Codex,
    Deepseek(DeepSeekModel),
}

#[derive(Debug, Subcommand)]
enum KeyCommand {
    /// Prompt for and save the DeepSeek API key.
    Set,
    /// Remove the saved DeepSeek API key.
    Delete,
}

#[derive(Debug, Subcommand)]
enum CredentialCommand {
    /// Print the key for Codex command-backed authentication.
    #[command(hide = true)]
    Print,
}

fn main() -> ExitCode {
    match run(Cli::parse(), &NativeCredentialStore) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run<S: CredentialStore>(cli: Cli, credentials: &S) -> Result<()> {
    let command = match cli.command {
        Some(command) => command,
        None => {
            let stdin = io::stdin();
            let stdout = io::stdout();
            match prompt_for_selection(&mut stdin.lock(), &mut stdout.lock())? {
                Selection::Codex => Command::Codex,
                Selection::Deepseek(model) => Command::Deepseek {
                    model: match model {
                        DeepSeekModel::Flash => DeepSeekModelArg::Flash,
                        DeepSeekModel::Pro => DeepSeekModelArg::Pro,
                    },
                },
            }
        }
    };

    match command {
        Command::Setup { model } => {
            if !credentials.contains_key()? {
                set_key(credentials)?;
            }
            activate_deepseek(credentials, model.into())?;
        }
        Command::Deepseek { model } => {
            activate_deepseek(credentials, model.into())?;
        }
        Command::Use {
            provider: Provider::Deepseek,
        } => {
            activate_deepseek(credentials, DeepSeekModel::Flash)?;
        }
        Command::Codex
        | Command::Use {
            provider: Provider::Codex,
        } => {
            let installations = discover_installations()?;
            for installation in &installations {
                if installation.companion && !installation.paths.state.exists() {
                    continue;
                }
                let report = config::switch_to_codex(&installation.paths)?;
                print_report(
                    "The saved Codex provider is active.",
                    &report,
                    installation,
                    installations.len(),
                );
            }
        }
        Command::Status => {
            let key_status = if credentials.contains_key()? {
                "stored in OS keychain"
            } else {
                "not set"
            };
            println!("Runtime: {}", runtime_name());
            println!("Credential backend: {}", credential_backend_name());
            println!("DeepSeek key: {key_status}");
            let installations = discover_installations()?;
            for installation in &installations {
                print_status(installation, installations.len())?;
            }
        }
        Command::Key {
            command: KeyCommand::Set,
        } => set_key(credentials)?,
        Command::Key {
            command: KeyCommand::Delete,
        } => {
            if credentials.delete()? {
                println!("DeepSeek API key removed from the OS keychain.");
            } else {
                println!("No DeepSeek API key was stored.");
            }
        }
        Command::Credential {
            command: CredentialCommand::Print,
        } => {
            let secret = credentials.get()?;
            let mut stdout = io::stdout().lock();
            stdout
                .write_all(secret.expose_secret().as_bytes())
                .map_err(|source| deepswitch::error::AppError::Io {
                    path: PathBuf::from("<stdout>"),
                    source,
                })?;
            stdout
                .flush()
                .map_err(|source| deepswitch::error::AppError::Io {
                    path: PathBuf::from("<stdout>"),
                    source,
                })?;
        }
    }
    Ok(())
}

fn activate_deepseek<S: CredentialStore>(credentials: &S, model: DeepSeekModel) -> Result<()> {
    credentials
        .contains_key()?
        .then_some(())
        .ok_or(deepswitch::error::AppError::CredentialMissing)?;
    let helper = current_executable()?;
    let installations = discover_installations()?;
    let message = format!("{} is active.", model.label());
    for installation in &installations {
        let report = config::switch_to_deepseek(&installation.paths, &helper, model)?;
        print_report(&message, &report, installation, installations.len());
    }
    Ok(())
}

fn discover_installations() -> Result<Vec<Installation>> {
    let current = CodexPaths::discover()?;
    let mut installations = vec![Installation {
        name: if is_wsl_runtime() { "WSL CLI" } else { "Codex" },
        paths: current.clone(),
        companion: false,
    }];

    if is_wsl_runtime()
        && std::env::var_os("CODEX_HOME").is_none()
        && let Ok(home) = windows_app_codex_home()
        && home != current.home
        && windows_wsl_agent_installed(&home)
    {
        installations.push(Installation {
            name: "Windows app (WSL agent)",
            paths: CodexPaths::from_codex_home(home),
            companion: true,
        });
    }

    Ok(installations)
}

fn windows_wsl_agent_installed(codex_home: &std::path::Path) -> bool {
    codex_home.join("bin").join("wsl").join("codex").is_file()
}

fn print_status(installation: &Installation, installation_count: usize) -> Result<()> {
    let status = config::status(&installation.paths)?;
    if installation_count > 1 {
        println!();
        println!("{}:", installation.name);
    }
    println!("Codex home: {}", installation.paths.home.display());
    println!("Active provider: {}", status.provider);
    println!(
        "Active model: {}",
        status.model.as_deref().unwrap_or("(default)")
    );
    println!(
        "DeepSeek configuration: {}",
        if status.deepseek_installed {
            "installed"
        } else {
            "not installed"
        }
    );
    println!(
        "Original Codex selection: {}",
        if status.original_state_saved {
            "saved"
        } else {
            "not saved"
        }
    );
    Ok(())
}

fn windows_app_codex_home() -> Result<PathBuf> {
    let windows_profile = command_output(
        ProcessCommand::new("cmd.exe").args(["/d", "/c", "echo", "%USERPROFILE%"]),
        "cmd.exe",
    )?;
    let windows_profile = windows_profile.trim();
    if windows_profile.is_empty() || windows_profile == "%USERPROFILE%" {
        return Err(AppError::WindowsAppCodexHomeUnavailable(
            "Windows did not return a user profile path".to_owned(),
        ));
    }

    let wsl_profile = command_output(
        ProcessCommand::new("wslpath").args(["-u", windows_profile]),
        "wslpath",
    )?;
    let wsl_profile = wsl_profile.trim();
    if wsl_profile.is_empty() {
        return Err(AppError::WindowsAppCodexHomeUnavailable(
            "wslpath returned an empty user profile path".to_owned(),
        ));
    }

    let profile = PathBuf::from(wsl_profile);
    if !profile.is_absolute() {
        return Err(AppError::WindowsAppCodexHomeUnavailable(format!(
            "wslpath returned a non-absolute path: {}",
            profile.display()
        )));
    }
    Ok(profile.join(".codex"))
}

fn command_output(command: &mut ProcessCommand, name: &str) -> Result<String> {
    let output = command
        .output()
        .map_err(|error| AppError::WindowsAppCodexHomeUnavailable(format!("{name}: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::WindowsAppCodexHomeUnavailable(format!(
            "{name} exited with {}{}",
            output.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn prompt_for_selection<R: BufRead, W: Write>(input: &mut R, output: &mut W) -> Result<Selection> {
    writeln!(output, "Choose a provider:")
        .and_then(|()| writeln!(output, "  1) Codex"))
        .and_then(|()| writeln!(output, "  2) DeepSeek V4 Flash"))
        .and_then(|()| writeln!(output, "  3) DeepSeek V4 Pro"))
        .map_err(|source| AppError::Io {
            path: PathBuf::from("<stdout>"),
            source,
        })?;

    loop {
        write!(output, "Selection: ")
            .and_then(|()| output.flush())
            .map_err(|source| AppError::Io {
                path: PathBuf::from("<stdout>"),
                source,
            })?;

        let mut selection = String::new();
        if input
            .read_line(&mut selection)
            .map_err(|source| AppError::Io {
                path: PathBuf::from("<stdin>"),
                source,
            })?
            == 0
        {
            return Err(AppError::InvalidProviderSelection);
        }

        match selection.trim().to_ascii_lowercase().as_str() {
            "1" | "codex" => return Ok(Selection::Codex),
            "2" | "flash" | "deepseek" | "deepseek-v4-flash" => {
                return Ok(Selection::Deepseek(DeepSeekModel::Flash));
            }
            "3" | "pro" | "deepseek-v4-pro" => {
                return Ok(Selection::Deepseek(DeepSeekModel::Pro));
            }
            _ => {
                writeln!(
                    output,
                    "Enter 1 for Codex, 2 for DeepSeek V4 Flash, or 3 for DeepSeek V4 Pro."
                )
                .map_err(|source| AppError::Io {
                    path: PathBuf::from("<stdout>"),
                    source,
                })?;
            }
        }
    }
}

fn set_key<S: CredentialStore>(credentials: &S) -> Result<()> {
    let api_key = rpassword::prompt_password("DeepSeek API key: ").map_err(|source| {
        deepswitch::error::AppError::Io {
            path: PathBuf::from("<terminal>"),
            source,
        }
    })?;
    let secret = SecretString::from(api_key);
    credentials.set(&secret)?;
    println!("DeepSeek API key saved in the OS keychain.");
    Ok(())
}

fn current_executable() -> Result<PathBuf> {
    std::env::current_exe().map_err(|source| deepswitch::error::AppError::Io {
        path: PathBuf::from("<current executable>"),
        source,
    })
}

fn print_report(
    message: &str,
    report: &ChangeReport,
    installation: &Installation,
    installation_count: usize,
) {
    let prefix = if installation_count > 1 {
        format!("{}: ", installation.name)
    } else {
        String::new()
    };
    if report.changed {
        println!("{prefix}{message}");
        if let Some(path) = &report.backup {
            println!("Backup: {}", path.display());
        }
    } else {
        println!("{prefix}{message} Configuration was already up to date.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_installed_windows_wsl_agent() {
        let directory = tempfile::tempdir().expect("temp directory");
        let codex = directory.path().join("bin").join("wsl").join("codex");
        std::fs::create_dir_all(codex.parent().expect("codex parent")).expect("create parent");
        std::fs::write(&codex, b"test").expect("create marker");

        assert!(windows_wsl_agent_installed(directory.path()));
    }

    #[test]
    fn prompt_selects_codex_by_number() {
        let mut input = &b"1\n"[..];
        let mut output = Vec::new();

        assert_eq!(
            prompt_for_selection(&mut input, &mut output).expect("selection"),
            Selection::Codex
        );
        assert!(
            String::from_utf8(output)
                .expect("prompt")
                .contains("Choose a provider")
        );
    }

    #[test]
    fn prompt_retries_and_accepts_flash_by_name() {
        let mut input = &b"invalid\ndeepseek\n"[..];
        let mut output = Vec::new();

        assert_eq!(
            prompt_for_selection(&mut input, &mut output).expect("selection"),
            Selection::Deepseek(DeepSeekModel::Flash)
        );
        assert!(
            String::from_utf8(output)
                .expect("prompt")
                .contains("Enter 1 for Codex, 2 for DeepSeek V4 Flash, or 3 for DeepSeek V4 Pro.")
        );
    }

    #[test]
    fn prompt_accepts_pro_by_number() {
        let mut input = &b"3\n"[..];
        let mut output = Vec::new();

        assert_eq!(
            prompt_for_selection(&mut input, &mut output).expect("selection"),
            Selection::Deepseek(DeepSeekModel::Pro)
        );
    }

    #[test]
    fn prompt_rejects_end_of_input() {
        let mut input = &b""[..];
        let mut output = Vec::new();

        assert!(matches!(
            prompt_for_selection(&mut input, &mut output),
            Err(AppError::InvalidProviderSelection)
        ));
    }
}
