use serde::Deserialize;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
pub struct PackageManifest {
    pub package_identifier: String,
    pub package_version: String,
    pub default_locale: String,
    pub manifest_type: String,
    pub manifest_version: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
pub struct Installer {
    pub architecture: Architecture,
    pub installer_url: String,
    pub installer_sha256: String,
    pub installer_type: Option<InstallerType>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
pub struct InstallerSwitches {
    pub silent: Option<String>,
    pub log: Option<String>,
    pub silent_with_progress: Option<String>,
    pub install_location: Option<String>,
    pub custom: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub enum InstallerType {
    Exe,
    Zip,
    Wix,
    Msix,
    Nullsoft,
    Inno,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Architecture {
    X86,
    X64,
    Arm,
    Arm64,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
pub struct InstallerManifest {
    pub package_identifier: String,
    pub package_version: String,
    pub installers: Vec<Installer>,
    pub install_modes: Option<Vec<String>>,
    pub installer_switches: Option<InstallerSwitches>,
    pub installer_type: Option<InstallerType>,
}
