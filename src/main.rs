use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

use clap::{Parser, Subcommand, ValueEnum};
use codex_deepseek_switcher::{
    config::{self, ChangeReport, CodexPaths},
    credentials::{CredentialStore, NativeCredentialStore, credential_backend_name, runtime_name},
    error::Result,
};
use secrecy::{ExposeSecret, SecretString};

#[derive(Debug, Parser)]
#[command(
    name = "codex-deepseek-switcher",
    version,
    about = "Safely switch Codex between OpenAI and DeepSeek"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Store a DeepSeek key securely and activate DeepSeek.
    Setup,
    /// Activate the selected provider.
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

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Provider {
    /// Restore the OpenAI/Codex selection captured during setup.
    Codex,
    /// Use DeepSeek V4 Flash through the Responses API.
    Deepseek,
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
    let paths = CodexPaths::discover()?;
    match cli.command {
        Command::Setup => {
            if !credentials.contains_key()? {
                set_key(credentials)?;
            }
            let helper = current_executable()?;
            let report = config::switch_to_deepseek(&paths, &helper)?;
            print_report("DeepSeek is active.", &report);
        }
        Command::Use {
            provider: Provider::Deepseek,
        } => {
            credentials
                .contains_key()?
                .then_some(())
                .ok_or(codex_deepseek_switcher::error::AppError::CredentialMissing)?;
            let helper = current_executable()?;
            let report = config::switch_to_deepseek(&paths, &helper)?;
            print_report("DeepSeek is active.", &report);
        }
        Command::Use {
            provider: Provider::Codex,
        } => {
            let report = config::switch_to_codex(&paths)?;
            print_report("The saved Codex provider is active.", &report);
        }
        Command::Status => {
            let status = config::status(&paths)?;
            let key_status = if credentials.contains_key()? {
                "stored in OS keychain"
            } else {
                "not set"
            };
            println!("Runtime: {}", runtime_name());
            println!("Codex home: {}", paths.home.display());
            println!("Credential backend: {}", credential_backend_name());
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
            println!("DeepSeek key: {key_status}");
            println!(
                "Original Codex selection: {}",
                if status.original_state_saved {
                    "saved"
                } else {
                    "not saved"
                }
            );
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
                .map_err(|source| codex_deepseek_switcher::error::AppError::Io {
                    path: PathBuf::from("<stdout>"),
                    source,
                })?;
            stdout
                .flush()
                .map_err(|source| codex_deepseek_switcher::error::AppError::Io {
                    path: PathBuf::from("<stdout>"),
                    source,
                })?;
        }
    }
    Ok(())
}

fn set_key<S: CredentialStore>(credentials: &S) -> Result<()> {
    let api_key = rpassword::prompt_password("DeepSeek API key: ").map_err(|source| {
        codex_deepseek_switcher::error::AppError::Io {
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
    std::env::current_exe().map_err(|source| codex_deepseek_switcher::error::AppError::Io {
        path: PathBuf::from("<current executable>"),
        source,
    })
}

fn print_report(message: &str, report: &ChangeReport) {
    if report.changed {
        println!("{message}");
        if let Some(path) = &report.backup {
            println!("Backup: {}", path.display());
        }
    } else {
        println!("{message} Configuration was already up to date.");
    }
}
