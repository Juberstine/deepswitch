use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
    process::ExitCode,
};

use clap::{Parser, Subcommand, ValueEnum};
use deepswitch::{
    config::{self, ChangeReport, CodexPaths},
    credentials::{CredentialStore, NativeCredentialStore, credential_backend_name, runtime_name},
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

#[derive(Debug, Subcommand)]
enum Command {
    /// Activate the saved OpenAI/Codex provider.
    Codex,
    /// Activate DeepSeek V4 Flash.
    Deepseek,
    /// Store a DeepSeek key securely and activate DeepSeek.
    Setup,
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
    let command = match cli.command {
        Some(command) => command,
        None => {
            let stdin = io::stdin();
            let stdout = io::stdout();
            match prompt_for_provider(&mut stdin.lock(), &mut stdout.lock())? {
                Provider::Codex => Command::Codex,
                Provider::Deepseek => Command::Deepseek,
            }
        }
    };

    match command {
        Command::Setup => {
            if !credentials.contains_key()? {
                set_key(credentials)?;
            }
            let helper = current_executable()?;
            let report = config::switch_to_deepseek(&paths, &helper)?;
            print_report("DeepSeek is active.", &report);
        }
        Command::Deepseek
        | Command::Use {
            provider: Provider::Deepseek,
        } => {
            credentials
                .contains_key()?
                .then_some(())
                .ok_or(deepswitch::error::AppError::CredentialMissing)?;
            let helper = current_executable()?;
            let report = config::switch_to_deepseek(&paths, &helper)?;
            print_report("DeepSeek is active.", &report);
        }
        Command::Codex
        | Command::Use {
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

fn prompt_for_provider<R: BufRead, W: Write>(input: &mut R, output: &mut W) -> Result<Provider> {
    writeln!(output, "Choose a provider:")
        .and_then(|()| writeln!(output, "  1) Codex"))
        .and_then(|()| writeln!(output, "  2) DeepSeek"))
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
            "1" | "codex" => return Ok(Provider::Codex),
            "2" | "deepseek" => return Ok(Provider::Deepseek),
            _ => {
                writeln!(output, "Enter 1 for Codex or 2 for DeepSeek.").map_err(|source| {
                    AppError::Io {
                        path: PathBuf::from("<stdout>"),
                        source,
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_selects_codex_by_number() {
        let mut input = &b"1\n"[..];
        let mut output = Vec::new();

        assert_eq!(
            prompt_for_provider(&mut input, &mut output).expect("provider"),
            Provider::Codex
        );
        assert!(
            String::from_utf8(output)
                .expect("prompt")
                .contains("Choose a provider")
        );
    }

    #[test]
    fn prompt_retries_and_accepts_deepseek_by_name() {
        let mut input = &b"invalid\ndeepseek\n"[..];
        let mut output = Vec::new();

        assert_eq!(
            prompt_for_provider(&mut input, &mut output).expect("provider"),
            Provider::Deepseek
        );
        assert!(
            String::from_utf8(output)
                .expect("prompt")
                .contains("Enter 1 for Codex or 2 for DeepSeek")
        );
    }

    #[test]
    fn prompt_rejects_end_of_input() {
        let mut input = &b""[..];
        let mut output = Vec::new();

        assert!(matches!(
            prompt_for_provider(&mut input, &mut output),
            Err(AppError::InvalidProviderSelection)
        ));
    }
}
