use anyhow::{anyhow, Context};
use reqwest::Url;
use serde::Deserialize;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum PresetsPath {
    LocalDir(PathBuf),
    LocalArchive(PathBuf),
    UrlArchive(Url),
    GitHttp(Url),
    GitSSH(Url),
}

impl std::str::FromStr for PresetsPath {
    type Err = String;

    // TODO: Improve error handling
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("http://") || s.starts_with("https://") {
            if s.ends_with(".zip") {
                Ok(Self::UrlArchive(Url::parse(s).map_err(|e| e.to_string())?))
            } else if (s.ends_with(".git")) {
                Ok(Self::GitHttp(Url::parse(s).map_err(|e| e.to_string())?))
            } else {
                Err(format!("Could not parse URL: {}", &s))
            }
        } else if (s.starts_with("git@") || s.starts_with("ssh://")) && s.ends_with(".git") {
            Ok(Self::GitSSH(Url::parse(s).map_err(|e| e.to_string())?))
        } else {
            // TODO: Check if valid path
            // TODO: Improve archive detection - check MIME ?
            if s.ends_with(".zip") {
                Ok(Self::LocalArchive(
                    PathBuf::from_str(s).map_err(|e| e.to_string())?,
                ))
            } else {
                Ok(Self::LocalDir(
                    PathBuf::from_str(s).map_err(|e| e.to_string())?,
                ))
            }
        }
    }
}

#[derive(Deserialize, Debug)]
struct Preset {
    packages: Option<Vec<String>>,
    script: Option<String>,
    environment_variables: Option<Vec<String>>,
    shared_directories: Option<Vec<PathBuf>>,
    aur_packages: Option<Vec<String>>,
}

fn visit_dirs(dir: &Path, filevec: &mut Vec<PathBuf>) -> Result<(), io::Error> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit_dirs(&path, filevec)?;
            } else if entry.path().extension() == Some(&std::ffi::OsString::from("toml")) {
                filevec.push(entry.path());
            }
        }
    }
    Ok(())
}

impl Preset {
    fn load(path: &Path) -> anyhow::Result<Self> {
        let data = fs::read_to_string(path).with_context(|| format!("{}", path.display()))?;
        toml::from_str(&data).with_context(|| format!("{}", path.display()))
    }

    fn process(
        &self,
        packages: &mut HashSet<String>,
        scripts: &mut Vec<Script>,
        environment_variables: &mut HashSet<String>,
        path: &Path,
        aur_packages: &mut HashSet<String>,
    ) -> anyhow::Result<()> {
        if let Some(preset_packages) = &self.packages {
            packages.extend(preset_packages.clone());
        }

        if let Some(preset_aur_packages) = &self.aur_packages {
            aur_packages.extend(preset_aur_packages.clone());
        }

        if let Some(preset_environment_variables) = &self.environment_variables {
            environment_variables.extend(preset_environment_variables.clone());
        }

        if let Some(script_text) = &self.script {
            scripts.push(Script {
                script_text: script_text.clone(),
                shared_dirs: self
                    .shared_directories
                    .clone()
                    .map(|x| {
                        // Convert directories to absolute paths
                        // If any shared directory is not a directory then throw an error
                        x.iter()
                            .cloned()
                            .map(|y| {
                                let full_path = path.parent().expect("Path has no parent").join(&y);
                                if full_path.is_dir() {
                                    Ok(full_path)
                                } else {
                                    Err(anyhow!(
                                        "Preset: {} - shared directory: {} is not directory",
                                        path.display(),
                                        y.display()
                                    ))
                                }
                            })
                            .collect::<anyhow::Result<Vec<_>>>()
                    })
                    .map_or(Ok(None), |r| r.map(Some))?,
            });
        }
        Ok(())
    }
}

pub struct Script {
    pub script_text: String,
    pub shared_dirs: Option<Vec<PathBuf>>,
}

pub struct PresetsCollection {
    pub packages: HashSet<String>,
    pub aur_packages: HashSet<String>,
    pub scripts: Vec<Script>,
}

impl PresetsCollection {
    pub fn load(list: &[PathBuf]) -> anyhow::Result<Self> {
        let mut packages = HashSet::new();
        let mut aur_packages = HashSet::new();
        let mut scripts: Vec<Script> = Vec::new();
        let mut environment_variables = HashSet::new();

        for preset in list {
            if preset.is_dir() {
                // Build vector of paths to files, then sort by path name
                // Recursively load directories of preset files
                let mut dir_paths: Vec<PathBuf> = Vec::new();
                visit_dirs(preset, &mut dir_paths)
                    .with_context(|| format!("{}", preset.display()))?;

                // Order not guaranteed so we sort
                // In the future may want to support numerical sort i.e. 15_... < 100_...
                dir_paths.sort();

                for path in dir_paths {
                    Preset::load(&path)?.process(
                        &mut packages,
                        &mut scripts,
                        &mut environment_variables,
                        &path,
                        &mut aur_packages,
                    )?;
                }
            } else {
                Preset::load(preset)?.process(
                    &mut packages,
                    &mut scripts,
                    &mut environment_variables,
                    preset,
                    &mut aur_packages,
                )?;
            }
        }
        let missing_envrionments: Vec<String> = environment_variables
            .into_iter()
            .filter(|var| env::var(var).is_err())
            .collect();

        if !missing_envrionments.is_empty() {
            return Err(anyhow!(
                "Missing environment variables {:?}",
                missing_envrionments
            ));
        }

        Ok(Self {
            packages,
            aur_packages,
            scripts,
        })
    }
}
