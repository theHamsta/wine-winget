use anyhow::{Context, Result, anyhow, bail};
use clap::{CommandFactory, Parser};
use log::{debug, info, warn};
use semver::{Version, VersionReq};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::cli::{Args, Install, Search};

mod cli;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct PackageManifest {
    pub package_identifier: String,
    pub package_version: String,
    pub default_locale: String,
    pub manifest_type: String,
    pub manifest_version: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct Installer {
    pub architecture: Architecture,
    pub installer_url: String,
    pub installer_sha256: String,
    pub installer_type: Option<InstallerType>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct InstallerSwitches {
    pub silent: Option<String>,
    pub log: Option<String>,
    pub silent_with_progress: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
enum InstallerType {
    Exe,
    Zip,
    Wix,
    Nullsoft,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Architecture {
    X86,
    X64,
    Arm64,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct InstallerManifest {
    pub package_identifier: String,
    pub package_version: String,
    pub installers: Vec<Installer>,
    pub install_modes: Option<Vec<String>>,
    pub installer_switches: Option<InstallerSwitches>,
    pub installer_type: Option<InstallerType>,
}

fn find_version(dir: &Path, req: Option<&VersionReq>) -> Result<PathBuf> {
    let mut versions = std::fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            debug!("Checking {path:?} as version");
            if path.is_dir()
                && let Some(filename) = path.file_name()
            {
                let version = Version::parse(&filename.to_string_lossy());
                if let Ok(version) = version
                    && req.is_none_or(|req| req.matches(&version))
                {
                    return Some((version, path));
                }
                // If not semver (e.g. major.minor) add implicit patch version
                let version = Version::parse(&(filename.to_string_lossy() + ".0"));
                if let Ok(version) = version
                    && req.is_none_or(|req| req.matches(&version))
                {
                    return Some((version, path));
                }
                warn!("Could not parse {path:?} as version. Ignoring!");
            }
            None
        })
        .collect::<Vec<_>>();
    versions.sort_unstable();

    versions
        .pop()
        .ok_or_else(|| anyhow!("Found no version"))
        .map(|(_version, path)| path)
        .inspect(|v| debug!("Found newest version: {v:?}"))
}

fn find_sub_case_insensitive(dir: &Path, subpath: &str, file: bool) -> Result<PathBuf> {
    let subdir = subpath.to_ascii_lowercase();
    std::fs::read_dir(dir)?
        .flatten()
        .find_map(|e| {
            let path = e.path();
            debug!("Checking {path:?} for {subdir:?}");
            if (if file { path.is_file() } else { path.is_dir() })
                && let Some(filename) = path.file_name()
                && *filename.to_ascii_lowercase() == *subdir
            {
                Some(path)
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow!("Failed to find subpath {subdir:?} in {dir:?}"))
}

fn find_subfile_case_insensitive(dir: &Path, subdir: &str) -> Result<PathBuf> {
    find_sub_case_insensitive(dir, subdir, true)
}

fn find_subdir_case_insensitive(dir: &Path, subdir: &str) -> Result<PathBuf> {
    find_sub_case_insensitive(dir, subdir, false)
}

fn package_path(package: &str, repo_path: &Path) -> Result<PathBuf> {
    let first_letter = package
        .chars()
        .next()
        .ok_or_else(|| anyhow!("Provide empty package name!"))?
        .to_string();
    let (vendor, package) = package
        .split_once('.')
        .ok_or_else(|| anyhow!("Package name {package:?} does not contain a `.`. Package name should be something like LLVM.LLVM"))?;
    let manifest_path = repo_path.join("manifests");
    debug!("manifests_path={manifest_path:?}");
    if !manifest_path.is_dir() {
        bail!(
            "{manifest_path:?} is not a directory! Please provide the local path to the git repo of https://github.com/microsoft/winget-pkgs"
        );
    }

    let letter_path = find_subdir_case_insensitive(&manifest_path, &first_letter)
        .with_context(|| "Failed to find letter dir")?;
    let vendor_path = find_subdir_case_insensitive(&letter_path, vendor)
        .with_context(|| "Failed to find vendor dir")?;
    let package_path = find_subdir_case_insensitive(&vendor_path, package)
        .with_context(|| "Failed to find package dir")?;
    Ok(package_path)
}

fn version_path(
    package: &str,
    repo_path: &Path,
    version_requirement: Option<&VersionReq>,
) -> Result<PathBuf> {
    let package_path = package_path(package, repo_path)?;
    let version_path = find_version(&package_path, version_requirement)
        .with_context(|| "Failed to find version dir")?;
    debug!("version_path={version_path:?}");
    Ok(version_path)
}

async fn install_package(
    _args: &Args,
    package: &str,
    install_args: &Install,
) -> anyhow::Result<()> {
    let version_req = install_args
        .version
        .as_ref()
        .map(|v| VersionReq::parse(v))
        .transpose()
        .with_context(|| anyhow!("Failed to parse version requirement"))?;
    let version_path = version_path(package, &install_args.repo_path, version_req.as_ref())?;

    let (vendor, package) = package
        .split_once('.')
        .ok_or_else(|| anyhow!("Package name {package:?} does not contain a `.`. Package name should be something like LLVM.LLVM"))?;
    let package_manifest =
        find_subfile_case_insensitive(&version_path, &format!("{vendor}.{package}.yaml"))?;

    let package_manifest: PackageManifest = yaml_serde::from_reader(File::open(&package_manifest)?)
        .with_context(|| {
            format!(
                "Failed to parse PackageManifest {package_manifest:?}:\n{}",
                std::fs::read_to_string(&package_manifest).unwrap_or_else(|_| "".to_string())
            )
        })?;

    debug!("PackageManifest: {package_manifest:?}");
    let installer_manifest = find_subfile_case_insensitive(
        &version_path,
        &format!("{vendor}.{package}.installer.yaml"),
    )?;
    let package_manifest: InstallerManifest =
        yaml_serde::from_reader(File::open(&installer_manifest)?).with_context(|| {
            format!(
                "Failed to parse InstallerManifest {installer_manifest:?}:\n{}",
                std::fs::read_to_string(&installer_manifest).unwrap_or_else(|_| "".to_string())
            )
        })?;
    debug!("InstallerManifest: {package_manifest:?}");

    let arch_string = cfg_select! {
        target_arch = "x86" => Architecture::X86,
        target_arch = "x86_64" => Architecture::X64,
        target_arch = "aarch64" => Architecture::Arm64,
        _ => bail!("Unsupported arch"),
    };

    let target_installer = package_manifest
        .installers
        .iter()
        .find(|i| i.architecture == arch_string)
        .ok_or_else(|| anyhow!("Could not find installer for architecture {arch_string:?}"))?;
    debug!("Using installer: {target_installer:?}");
    println!("Downloading {:?}", target_installer.installer_url);

    let last = target_installer
        .installer_url
        .rsplit_once("/")
        .ok_or_else(|| anyhow!("Installer URL does not contain `/`"))?
        .1;
    let download_path = format!("/tmp/{last}");
    download_file(&target_installer.installer_url, &download_path).await?;
    let actual = sha256_string(&download_path)?.to_ascii_lowercase();
    let expected = target_installer.installer_sha256.to_ascii_lowercase();
    if actual != expected {
        bail!("Failed to verify checksum: actual {actual:?}, expected {expected:?}");
    }

    info!("Checksum ok");
    println!("Running {last:?}!");
    let mut install_cmd = if cfg!(unix) {
        std::process::Command::new("wine")
            .arg(&download_path)
            .spawn()?
    } else {
        std::process::Command::new(&download_path).spawn()?
    };
    let output = install_cmd.wait()?;
    if !output.success() {
        bail!("Installer failed!");
    }
    println!("Installer ran successfully!");
    let _ = std::fs::remove_file(&download_path);

    Ok(())
}

async fn install(args: &Args, install_args: &Install) -> Result<()> {
    for package in install_args.packages.iter() {
        install_package(args, package, install_args).await?;
    }

    Ok(())
}

fn search(_args: &Args, search_args: &Search) -> Result<()> {
    let search_string = &search_args.search_string.to_ascii_lowercase();

    let manifest_path = search_args.repo_path.join("manifests");
    debug!("manifests_path={manifest_path:?}");
    let mut todos = vec![(manifest_path, 0, false)];
    while let Some((todo_path, depth, match_all)) = todos.pop() {
        if depth > 2 {
            continue;
        }
        for e in std::fs::read_dir(&todo_path)?.flatten() {
            let path = e.path();
            if path.is_dir()
                && (path
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains(search_string)
                    || match_all)
            {
                match depth {
                    1 => {
                        todos.push((path.clone(), depth + 1, true));
                        println!(
                            "Found vendor: {}",
                            path.file_name()
                                .expect("Folder without name")
                                .to_string_lossy()
                        );
                    }
                    2 => {
                        todos.push((path.clone(), depth + 1, true));
                        let package = path.file_name().expect("Folder without name");
                        let vendor = path
                            .parent()
                            .expect("Recursion depth is 2, but no parent")
                            .file_name()
                            .expect("Folder without name");
                        println!(
                            "Found package: {}.{}",
                            vendor.to_string_lossy(),
                            package.to_string_lossy()
                        );
                    }
                    _ => (),
                }
            } else {
                if path.is_dir() {
                    todos.push((path.clone(), depth + 1, false));
                }
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        // Safety: is safe as no other threads launched yet
        unsafe { std::env::set_var("RUST_LOG", "warn") }
    }
    pretty_env_logger::init();
    let args = Args::parse();

    if let Some(shell) = args.shell_completion {
        let mut cmd = Args::command();
        let bin_name = cmd.get_name().to_string();

        clap_complete::generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
        return Ok(());
    }

    match args.command.as_ref() {
        Some(cli::Commands::Install(install_args)) => install(&args, install_args).await?,
        Some(cli::Commands::Upgrade(args)) => todo!(),
        Some(cli::Commands::Remove(args)) => todo!(),
        Some(cli::Commands::Search(search_args)) => search(&args, search_args)?,
        None => {
            cli::Args::command().print_help()?;
        }
    }

    Ok(())
}

/// Downloads the content from a URL and writes it to the specified path.
async fn download_file(url: &str, path: &str) -> Result<()> {
    // For robust, large file downloads, streaming is best.
    // Here, we use reqwest's async capabilities and write to a file.
    let client = reqwest::Client::new();
    let response = client.get(url).send().await?.error_for_status()?;

    let file = File::create(path)?;
    let mut writer = io::BufWriter::new(file);

    // Stream the body contents directly to the file writer
    let bytes = response.bytes().await?;
    writer.write_all(&bytes)?;
    writer.flush()?;

    Ok(())
}

/// Reads the file and calculates its SHA-256 hash, comparing it to the expected value.
fn sha256_string(path: &str) -> Result<String> {
    // 1. Open the file to calculate the hash
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();

    // Read the file chunk by chunk and update the hasher
    let mut buffer = [0; 8192]; // Read in 8KB chunks
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break; // End of file reached
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let result = hasher.finalize();
    Ok(hex::encode(result))
}
