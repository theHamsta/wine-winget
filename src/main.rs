use crate::schema::{Architecture, InstallerManifest, InstallerType, PackageManifest};
use anyhow::{Context, Result, anyhow, bail};
use clap::{CommandFactory, Parser};
use indicatif::ProgressBar;
use itertools::Itertools;
use log::{debug, info, trace, warn};
use regex::Regex;
use semver::{Version, VersionReq};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use crate::cli::{Args, Install, Search};

mod cli;
mod schema;

struct DeleteOnDrop<'path> {
    path: &'path Path,
}
impl<'path> DeleteOnDrop<'path> {
    fn new(path: &'path Path) -> Self {
        DeleteOnDrop { path }
    }
}

impl Drop for DeleteOnDrop<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.path);
    }
}

static QUADRUPLE_VERSION_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d+[.]\d+[.]\d+)[.]\d+").unwrap());

static LEADING_ZERO_VERSION_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[.]0+(\d+)").unwrap());

fn decompress_zip(reader: impl Read + Seek, target_path: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(reader)?;

    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        if file.is_file()
            && let Some(filename) = PathBuf::from(file.name()).file_name()
        {
            let out_path = target_path.join(filename);
            let mut out = File::create(&out_path)
                .with_context(|| format!("Failed to create {out_path:?} for extractiong zip"))?;
            println!("Decompressing {:?} to {out_path:?}", file.name());
            std::io::copy(&mut file, &mut out)?;
        }
    }

    Ok(())
}

fn find_version(dir: &Path, req: Option<&VersionReq>) -> Result<(Version, PathBuf)> {
    let mut versions = std::fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if matches!(
                path.file_name().unwrap().to_string_lossy().chars().next(),
                Some('A'..='Z') | Some('a'..='z')
            ) {
                warn!(
                    "Ignoring variant subpath {path:?} as a version. Append `.{}` to package name to install this variant",
                    path.file_name().unwrap().to_string_lossy()
                );
                return None;
            }
            debug!("Checking {path:?} as version");
            if path.is_dir()
                && let Some(filename) = path.file_name()
            {
                let version_str = filename.to_string_lossy();
                let version_str = QUADRUPLE_VERSION_REGEX.replace(&version_str, "$1");
                let version_str = LEADING_ZERO_VERSION_REGEX.replace_all(&version_str, ".$1");
                trace!("{filename:?} is version {version_str}");

                let version = Version::parse(&version_str);
                if let Ok(version) = &version
                    && req.is_none_or(|req| req.matches(version))
                {
                    return Some((version.clone(), path));
                }
                if version.is_err() {
                    // If not semver (e.g. major.minor) add implicit patch version
                    let version = Version::parse(&(version_str + ".0"));
                    if let Ok(version) = &version
                        && req.is_none_or(|req| req.matches(version))
                    {
                        return Some((version.clone(), path));
                    } else if version.is_err() {
                        warn!("Could not parse {path:?} as version. Ignoring! If this is a variant append `.{}` to package name",
                              path.file_name().unwrap().to_string_lossy());
                    }
                }
            }
            None
        })
        .collect::<Vec<_>>();
    versions.sort_unstable();

    versions
        .pop()
        .ok_or_else(|| anyhow!("Found no version"))
        .inspect(|(version, _)| println!("Found newest matching version: {version}"))
}

fn find_sub_case_insensitive(dir: &Path, subpath: &str, file: bool) -> Option<PathBuf> {
    let subdir = subpath.to_ascii_lowercase();
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
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
}

fn find_subfile_case_insensitive(dir: &Path, subdir: &str) -> Option<PathBuf> {
    find_sub_case_insensitive(dir, subdir, true)
}

fn find_subdir_case_insensitive(dir: &Path, subdir: &str) -> Option<PathBuf> {
    find_sub_case_insensitive(dir, subdir, false)
}

fn package_path(package: &str, repo_path: &Path) -> Result<PathBuf> {
    let first_letter = package
        .chars()
        .next()
        .ok_or_else(|| anyhow!("Provide empty package name!"))?
        .to_string();
    let manifest_path = repo_path.join("manifests");
    debug!("manifests_path={manifest_path:?}");
    if !manifest_path.is_dir() {
        bail!(
            "{manifest_path:?} is not a directory! Please provide the local path to the git repo of https://github.com/microsoft/winget-pkgs"
        );
    }

    let letter_path = find_subdir_case_insensitive(&manifest_path, &first_letter)
        .ok_or_else(|| anyhow!("Failed to find letter dir"))?;

    let mut package_path = letter_path;
    for split in package.split('.') {
        package_path = find_subdir_case_insensitive(&package_path, split)
            .ok_or_else(|| anyhow!("Failed to find subdir {split:?} in {package_path:?}"))?;
    }
    Ok(package_path)
}

fn version_path(
    package: &str,
    repo_path: &Path,
    version_requirement: Option<&VersionReq>,
) -> Result<(Version, PathBuf)> {
    let package_path = package_path(package, repo_path)?;
    let (version, version_path) = find_version(&package_path, version_requirement)
        .with_context(|| "Failed to find version dir")?;
    debug!("version_path={version_path:?}");
    Ok((version, version_path))
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
    let (_version, version_path) =
        version_path(package, &install_args.repo_path, version_req.as_ref())?;

    let package_manifest = find_subfile_case_insensitive(&version_path, &format!("{package}.yaml"))
        .ok_or_else(|| anyhow!("Could not find package_manifest"))?;

    let package_manifest: PackageManifest = yaml_serde::from_reader(File::open(&package_manifest)?)
        .with_context(|| {
            format!(
                "Failed to parse PackageManifest {package_manifest:?}:\n{}",
                std::fs::read_to_string(&package_manifest).unwrap_or_else(|_| "".to_string())
            )
        })?;

    debug!("PackageManifest: {package_manifest:?}");
    let installer_manifest =
        find_subfile_case_insensitive(&version_path, &format!("{package}.installer.yaml"))
            .ok_or_else(|| anyhow!("Failed to find installer manifest"))?;
    let package_manifest: InstallerManifest =
        yaml_serde::from_reader(File::open(&installer_manifest)?).with_context(|| {
            format!(
                "Failed to parse InstallerManifest {installer_manifest:?}:\n{}",
                std::fs::read_to_string(&installer_manifest).unwrap_or_else(|_| "".to_string())
            )
        })?;
    debug!("InstallerManifest: {package_manifest:?}");

    let arch = cfg_select! {
        target_arch = "x86" => Architecture::X86,
        target_arch = "x86_64" => Architecture::X64,
        target_arch = "aarch64" => Architecture::Arm64,
    };
    let fallback_arch = cfg_select! {
        target_arch = "x86" => Architecture::X86,
        target_arch = "x86_64" => Architecture::X86,
        target_arch = "aarch64" => Architecture::x86_64,
    };

    let target_installer = [arch, fallback_arch]
        .iter()
        .cartesian_product(package_manifest.installers.iter())
        .find(|&(&arch, i)| {
            (i.architecture == arch || i.architecture == Architecture::Neutral)
                && !matches!(i.installer_type, Some(InstallerType::Msix))
                && !matches!(i.installer_type, Some(InstallerType::Zip))
        })
        .map(|(_, installer)| installer)
        .ok_or_else(|| {
            anyhow!(
                "Could not find installer for architecture {arch:?} or fallback {fallback_arch:?}"
            )
        })?;

    debug!("Using installer: {target_installer:?}");
    println!("Downloading {:?}", target_installer.installer_url);

    let last = target_installer
        .installer_url
        .rsplit_once("/")
        .ok_or_else(|| anyhow!("Installer URL does not contain `/`"))?
        .1;
    let download_path = std::env::temp_dir().join(last);
    DeleteOnDrop::new(&download_path);
    download_file(&target_installer.installer_url, &download_path).await?;
    let actual = sha256_string(&download_path)?.to_ascii_lowercase();
    let expected = target_installer.installer_sha256.to_ascii_lowercase();
    if actual != expected {
        bail!("Failed to verify checksum: actual {actual:?}, expected {expected:?}");
    }

    info!("Checksum ok");
    debug!("{:?}", target_installer);
    if matches!(target_installer.installer_type, Some(InstallerType::Zip))
        || target_installer.installer_url.ends_with("zip")
    {
        let install_path = if cfg!(unix) {
            use std::process::Stdio;

            println!("Running `winepath c:/windows/system32`");
            let cmd = std::process::Command::new(&install_args.wine)
                .stdout(Stdio::piped())
                .arg("winepath")
                .arg("c:/windows/system32")
                .spawn()?;
            let output = cmd.wait_with_output()?;
            PathBuf::from(str::from_utf8(output.stdout.trim_ascii())?)
        } else {
            warn!(
                "Writing to {:?}. Please copy `.exe` manually to preferred location",
                std::env::temp_dir()
            );
            std::env::temp_dir()
        };

        decompress_zip(
            File::open(download_path).with_context(|| "Failed to open downloaded file")?,
            &install_path,
        )?;
    } else {
        println!("Running {last:?}!");
        let mut install_cmd = if cfg!(unix) {
            std::process::Command::new(&install_args.wine)
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
    }

    Ok(())
}

async fn install(args: &Args, install_args: &Install) -> Result<()> {
    if !install_args.no_update {
        println!("Updating {:?}", install_args.repo_path);
        let result = Command::new("git")
            .arg("-C")
            .arg(&install_args.repo_path)
            .arg("pull")
            .spawn();
        if let Err(error) = &result {
            warn!("Failed to update git repo: {error}");
        } else {
            if let Err(error) = result.unwrap().wait() {
                warn!("Failed to update git repo: {error}");
            }
        }
    }

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
                    0 => {
                        todos.push((path.clone(), depth + 1, false));
                    }
                    1 => {
                        todos.push((path.clone(), depth + 1, true));
                        println!(
                            "Found vendor: {}",
                            path.file_name()
                                .expect("Folder without name")
                                .to_string_lossy()
                        );
                    }
                    2.. => {
                        todos.push((path.clone(), depth + 1, true));
                        let package = path.file_name().expect("Folder without name");
                        if !package.to_string_lossy().contains(".") {
                            let mut segments = VecDeque::new();

                            let mut cur = path.clone();
                            for _ in 0..depth {
                                let seg = cur.file_name().unwrap().to_string_lossy().to_string();
                                segments.push_front(seg);
                                cur = cur.parent().unwrap().to_owned();
                            }

                            println!("Found package: {}", segments.make_contiguous().join("."));
                        }
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
        Some(cli::Commands::Upgrade(_args)) => todo!(),
        Some(cli::Commands::Remove(_args)) => todo!(),
        Some(cli::Commands::Search(search_args)) => search(&args, search_args)?,
        None => {
            cli::Args::command().print_help()?;
        }
    }

    Ok(())
}

/// Downloads the content from a URL and writes it to the specified path.
async fn download_file(url: &str, path: &Path) -> Result<()> {
    let client = reqwest::Client::new();

    let mut response = client.get(url).send().await?;
    let content_length = response.content_length().unwrap_or(0);

    let progress_bar = ProgressBar::new(content_length);
    let mut file = File::create(path)?;
    let mut bytes_received = 0u64;

    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk)?;
        bytes_received += chunk.len() as u64;
        progress_bar.set_position(bytes_received);
    }

    Ok(())
}

/// Reads the file and calculates its SHA-256 hash, comparing it to the expected value.
fn sha256_string(path: &Path) -> Result<String> {
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
